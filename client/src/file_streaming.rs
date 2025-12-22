use anyhow::{anyhow, Context, Result};
use backup_sync_protocol::FileOperation;
use blake3::Hasher;
use librsync::whole::{delta, patch};
use std::fs::File;
use std::io::{self, BufReader, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use tempfile::NamedTempFile;
use tracing::{info, instrument};

// A helper struct to calculate Hash while reading (Pass-through Reader)
struct HashingReader<R> {
    inner: R,
    hasher: Hasher,
}

impl<R: Read> HashingReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            hasher: Hasher::new(),
        }
    }
    fn finalize(&self) -> String {
        self.hasher.finalize().to_hex().to_string()
    }
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.hasher.update(&buf[..n]);
        }
        Ok(n)
    }
}

const CHUNK_SIZE: usize = 64 * 1024; // 64KB

/// A custom Writer that chunks incoming data and sends it to a channel
pub struct ChunkedDeltaWriter {
    buffer: Vec<u8>,
    chunk_size: usize,
    transfer_id: u64,
    chunk_counter: u64,
    // We use blocking_send because this runs in a Rayon thread
    sender: mpsc::Sender<FileOperation>,
}

impl ChunkedDeltaWriter {
    pub fn new(transfer_id: u64, chunk_size: usize, sender: mpsc::Sender<FileOperation>) -> Self {
        Self {
            buffer: Vec::with_capacity(chunk_size),
            chunk_size,
            transfer_id,
            chunk_counter: 0,
            sender,
        }
    }

    fn flush_chunk(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        let chunk_data = std::mem::replace(&mut self.buffer, Vec::with_capacity(self.chunk_size));

        let msg = FileOperation::FileChunk {
            transfer_id: self.transfer_id,
            chunk_index: self.chunk_counter,
            data: chunk_data,
        };

        self.chunk_counter += 1;

        // Block the Rayon thread until the channel has space (backpressure)
        self.sender
            .send(msg)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Receiver dropped"))?;

        Ok(())
    }
}

impl Write for ChunkedDeltaWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut written = 0;

        // Simple buffering logic
        while written < buf.len() {
            let space_left = self.chunk_size - self.buffer.len();
            let to_copy = space_left.min(buf.len() - written);

            self.buffer
                .extend_from_slice(&buf[written..written + to_copy]);
            written += to_copy;

            if self.buffer.len() == self.chunk_size {
                self.flush_chunk()?;
            }
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flush_chunk()
    }
}

#[instrument(skip(signature_data, tx))]
pub fn generate_delta_streamed(
    path: PathBuf,
    signature_data: Vec<u8>,
    transfer_id: u64,
    tx: mpsc::Sender<FileOperation>,
) -> Result<()> {
    // 1. Load the Signature
    // librsync needs the signature in RAM. This is usually fine (sig is ~1% of file size)
    let mut signature = Cursor::new(signature_data);

    // 2. Open File & Setup Hashing
    let file = File::open(&path).with_context(|| format!("Failed to open file: {path:?}"))?;
    let mut reader = HashingReader::new(BufReader::new(file));

    // 4. Check file size to decide strategy
    let metadata = std::fs::metadata(&path)
        .with_context(|| format!("Failed to get metadata for: {path:?}"))?;
    let file_size = metadata.len();

    // OPTIMIZATION: For small files, compute delta in memory and send a single ApplyDelta message
    // This avoids the overhead of StartTransfer -> Chunks -> EndTransfer
    if file_size < CHUNK_SIZE as u64 {
        let mut delta_buffer = Vec::new();
        delta(&mut signature, &mut reader, &mut delta_buffer)
            .map_err(|e| anyhow!("Failed to compute delta: {e}"))?;

        let final_hash = reader.finalize();

        return tx
            .send(FileOperation::ApplyDelta {
                transfer_id,
                relative_path: path,
                delta: delta_buffer,
                expected_hash: final_hash,
            })
            .context("Problem by sending ApplyDelta");
    }

    // 5. Setup our Streaming Writer
    let mut writer = ChunkedDeltaWriter::new(transfer_id, CHUNK_SIZE, tx.clone()); // 64KB chunks

    // 6. Send "StartTransfer" message
    tx.send(FileOperation::StartTransfer {
        transfer_id,
        relative_path: path.clone(), // Simplify path as needed
        total_size: file_size,
    })
    .context("Problem by sending StartTransfer")?;

    // 7. Compute Delta (The Heavy Lift)
    // The reader feeds data to librsync, librsync feeds delta to our writer
    delta(&mut signature, &mut reader, &mut writer)
        .map_err(|e| anyhow!("Failed to compute delta: {e}"))?;

    // 8. Finalize
    writer.flush().context("Failed to send final chunk")?; // Ensure last chunk is sent
    let final_hash = reader.finalize();

    // 9. Send Completion / Integrity Message
    // We send a special "Last Chunk" or a specific "End" message containing the Hash.
    tx.send(FileOperation::EndTransfer {
        transfer_id,
        expected_hash: final_hash,
    })
    .with_context(|| format!("Failed to send EndTransfer: {path:?}"))?;

    Ok(())
}

#[instrument(skip(delta))]
pub fn apply_delta_securely(
    base_path: &Path,
    relative_path: &Path,
    delta: Vec<u8>,
    expected_hash: String,
) -> Result<()> {
    // 1. Construct full path
    let target_file_path = base_path.join(relative_path);

    if !target_file_path.exists() {
        return Err(anyhow!("Basis file not found: {target_file_path:?}"));
    }

    // 2. Open the "Basis" file (the current local version)
    let basis_file = File::open(&target_file_path)?;
    let mut basis_reader = BufReader::new(basis_file);

    // 3. Create a Temporary File in the SAME directory
    // We use the same dir to ensure the final rename is atomic (same filesystem)
    let parent_dir = target_file_path.parent().unwrap_or(base_path);
    let mut temp_file = NamedTempFile::new_in(parent_dir)?;

    // 4. Apply the Patch (Librsync logic)
    // Note: librsync usually takes a Read stream (basis), Read stream (delta), and Write stream (output)
    let mut delta_reader = Cursor::new(delta);

    // WRAPPER NOTE: Replace this with your specific librsync-rs syntax
    // e.g., librsync::apply(&mut basis_reader, &mut delta_reader, &mut temp_file)?;
    patch(&mut basis_reader, &mut delta_reader, &mut temp_file)
        .map_err(|e| anyhow!("Failed to apply patch: {e}"))?;

    // 5. Verify Integrity (Hash the temp file)
    // Rewind temp file to read it for hashing
    let mut temp_file_ro = temp_file.reopen()?;
    let computed_hash = compute_blake3_hash(&mut temp_file_ro)?;

    if computed_hash != expected_hash {
        // If hash fails, the temp file is dropped automatically here (deleted)
        return Err(anyhow!(
            "Integrity check failed: expected {expected_hash}, got {computed_hash}"
        ));
    }

    // 6. Atomic Commit
    // This replaces the old file with the new one instantly
    temp_file.persist(&target_file_path).map_err(|e| e.error)?;

    info!("Successfully patched and verified: {:?}", relative_path);
    Ok(())
}

/// Helper: Compute Blake3 Hash
fn compute_blake3_hash(reader: &mut impl Read) -> anyhow::Result<String> {
    let mut hasher = blake3::Hasher::new();
    std::io::copy(reader, &mut hasher)?;
    Ok(hasher.finalize().to_hex().to_string())
}
