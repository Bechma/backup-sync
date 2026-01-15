use super::{temp_folder_path, Hashing};
use crate::models::RelativePath;
use crate::protocol::DeltaSyncOp;
use anyhow::{anyhow, bail, Context, Result};
use blake3::Hash;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Debug)]
struct DeltaTransferState {
    path: RelativePath,
    total_chunks: usize,
    received_chunks: Vec<Option<Vec<libsync3::DeltaCommand>>>,
    pending_end: Option<Hash>,
}

#[derive(Debug)]
pub struct DeltaSyncProcessor {
    id: String,
    path: PathBuf,
    transfer_states: Arc<Mutex<HashMap<u64, DeltaTransferState>>>,
}

impl DeltaSyncProcessor {
    pub fn new(id: String, path: PathBuf) -> Self {
        Self {
            id,
            path,
            transfer_states: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[inline]
    fn resolve(&self, path: &RelativePath) -> PathBuf {
        path.resolve(&self.path)
    }

    fn lock_transfer_states(&self) -> Result<MutexGuard<'_, HashMap<u64, DeltaTransferState>>> {
        self.transfer_states
            .lock()
            .map_err(|e| anyhow!("Transfer states mutex poisoned: {e}"))
    }

    fn temp_path_ref(&self, id: u64) -> PathBuf {
        temp_folder_path(&self.id).join(format!("{id}_delta.tmp"))
    }

    pub fn process_delta_sync(&self, op: DeltaSyncOp) -> Result<Option<DeltaSyncOp>> {
        match op {
            DeltaSyncOp::RequestSignature { path } => self.handle_request_signature(&path),

            DeltaSyncOp::ResponseSignature { .. } => {
                // This is typically handled by the origin side, not the backup
                Ok(None)
            }

            DeltaSyncOp::DeltaTransferStart {
                id,
                path,
                total_chunks,
            } => self.handle_delta_start(id, &path, total_chunks),

            DeltaSyncOp::DeltaTransferChunk {
                id,
                chunk_index,
                chunk,
            } => self.handle_delta_chunk(id, chunk_index, chunk),

            DeltaSyncOp::DeltaTransferEnd { id, hash } => self.handle_delta_end(id, hash),

            DeltaSyncOp::Abort { id, reason } => {
                println!("TODO: replace this println! Delta Abort: {reason}");
                let _ = fs::remove_file(self.temp_path_ref(id));
                self.lock_transfer_states()?.remove(&id);
                Ok(None)
            }
        }
    }

    fn handle_request_signature(&self, path: &RelativePath) -> Result<Option<DeltaSyncOp>> {
        let resolved_path = self.resolve(path);

        if !resolved_path.exists() {
            // File doesn't exist, return empty signature (use block size of 1024 as default)
            return Ok(Some(DeltaSyncOp::ResponseSignature {
                path: path.clone(),
                signature: libsync3::Signatures::new(1024),
            }));
        }

        let file = fs::File::open(&resolved_path)
            .with_context(|| format!("Failed to open file: {}", resolved_path.display()))?;

        let signature = libsync3::generate_signatures(file)
            .map_err(|e| anyhow!("Failed to generate signature: {e}"))?;

        Ok(Some(DeltaSyncOp::ResponseSignature {
            path: path.clone(),
            signature,
        }))
    }

    fn handle_delta_start(
        &self,
        id: u64,
        path: &RelativePath,
        total_chunks: usize,
    ) -> Result<Option<DeltaSyncOp>> {
        // Ensure temp directory exists
        let temp_folder = temp_folder_path(&self.id);
        fs::create_dir_all(&temp_folder)?;

        // Initialize transfer state with pre-allocated slots
        let mut received_chunks = Vec::with_capacity(total_chunks);
        for _ in 0..total_chunks {
            received_chunks.push(None);
        }

        let mut states = self.lock_transfer_states()?;
        states.insert(
            id,
            DeltaTransferState {
                path: path.clone(),
                total_chunks,
                received_chunks,
                pending_end: None,
            },
        );

        Ok(None)
    }

    fn handle_delta_chunk(
        &self,
        id: u64,
        chunk_index: u64,
        chunk: Vec<libsync3::DeltaCommand>,
    ) -> Result<Option<DeltaSyncOp>> {
        let mut states = self.lock_transfer_states()?;
        let state = states.get_mut(&id).context("Delta transfer not started")?;

        // Validate chunk_index is within bounds
        let idx = chunk_index as usize;
        if idx >= state.total_chunks {
            bail!(
                "Chunk index {} out of bounds (total_chunks: {})",
                chunk_index,
                state.total_chunks
            );
        }

        // Check if chunk was already received (duplicate)
        // if state.received_chunks[idx].is_some() {
        //     bail!("Duplicate chunk received at index {}", chunk_index);
        // }

        // Insert chunk at its correct position
        state.received_chunks[idx] = Some(chunk);

        // Check if we have pending end and all chunks are now received
        if let Some(expected_hash) = state.pending_end
            && state.received_chunks.iter().all(|c| c.is_some())
        {
            let path = state.path.clone();
            let chunks: Vec<_> = state.received_chunks.drain(..).flatten().collect();
            drop(states);
            return self.apply_delta(id, &path, chunks, expected_hash);
        }

        Ok(None)
    }

    fn handle_delta_end(&self, id: u64, expected_hash: Hash) -> Result<Option<DeltaSyncOp>> {
        let mut states = self.lock_transfer_states()?;
        let state = states.get_mut(&id).context("Transfer state not found")?;

        let all_received = state.received_chunks.iter().all(|c| c.is_some());
        if !all_received {
            state.pending_end = Some(expected_hash);
            return Ok(None);
        }

        let path = state.path.clone();
        let chunks: Vec<_> = state.received_chunks.drain(..).flatten().collect();
        drop(states);

        self.apply_delta(id, &path, chunks, expected_hash)
    }

    fn apply_delta(
        &self,
        id: u64,
        path: &RelativePath,
        chunks: Vec<Vec<libsync3::DeltaCommand>>,
        expected_hash: Hash,
    ) -> Result<Option<DeltaSyncOp>> {
        let resolved_path = self.resolve(path);
        let temp_path = self.temp_path_ref(id);

        // Flatten all delta commands
        let delta: Vec<libsync3::DeltaCommand> = chunks.into_iter().flatten().collect();

        // Ensure temp directory exists
        if let Some(parent) = temp_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Create temp file with hashing writer - streams directly to disk while computing hash
        let temp_file = File::create(&temp_path)
            .with_context(|| format!("Failed to create temp file: {}", temp_path.display()))?;
        let mut hashing_writer = Hashing::new(BufWriter::new(temp_file));

        // Apply delta: streams from original file to temp file
        // libsync3::apply_delta requires Read + Seek, so we handle both cases
        if resolved_path.exists() {
            let original_file = File::open(&resolved_path).with_context(|| {
                format!("Failed to open original file: {}", resolved_path.display())
            })?;
            let original_reader = BufReader::new(original_file);
            libsync3::apply_delta(original_reader, &delta, &mut hashing_writer)
                .map_err(|e| anyhow!("Failed to apply delta: {e}"))?;
        } else {
            self.lock_transfer_states()?.remove(&id);
            bail!("File does not exist: {}", resolved_path.display());
        };

        // Verify hash
        let actual_hash = hashing_writer.finalize();
        if actual_hash != expected_hash {
            let _ = fs::remove_file(&temp_path);
            self.lock_transfer_states()?.remove(&id);
            bail!("Hash mismatch after delta apply: expected {expected_hash}, got {actual_hash}");
        }

        // Atomic move to final location
        if let Some(parent) = resolved_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&temp_path, &resolved_path)?;

        // Clean up transfer state
        self.lock_transfer_states()?.remove(&id);

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_test_processor() -> (DeltaSyncProcessor, tempfile::TempDir) {
        let temp_dir = tempdir().unwrap();
        let processor = DeltaSyncProcessor::new(
            uuid::Uuid::new_v4().to_string(),
            temp_dir.path().to_path_buf(),
        );
        (processor, temp_dir)
    }

    fn path_to_transfer_id(path: &RelativePath) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        path.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn test_request_signature_nonexistent_file() -> Result<()> {
        let (processor, _temp) = create_test_processor();
        let path = RelativePath::new("nonexistent.txt")?;

        let result =
            processor.process_delta_sync(DeltaSyncOp::RequestSignature { path: path.clone() })?;

        match result {
            Some(DeltaSyncOp::ResponseSignature {
                path: resp_path,
                signature,
            }) => {
                assert_eq!(resp_path, path);
                assert_eq!(signature.len(), 0);
            }
            _ => bail!("Expected ResponseSignature"),
        }

        Ok(())
    }

    #[test]
    fn test_request_signature_existing_file() -> Result<()> {
        let (processor, temp) = create_test_processor();
        let file_path = temp.path().join("test.txt");
        fs::write(&file_path, "Hello, World!")?;

        let path = RelativePath::new("test.txt")?;
        let result =
            processor.process_delta_sync(DeltaSyncOp::RequestSignature { path: path.clone() })?;

        match result {
            Some(DeltaSyncOp::ResponseSignature {
                path: resp_path,
                signature,
            }) => {
                assert_eq!(resp_path, path);
                assert!(signature.len() > 0);
            }
            _ => bail!("Expected ResponseSignature"),
        }

        Ok(())
    }

    #[test]
    fn test_delta_transfer_complete() -> Result<()> {
        let (processor, temp) = create_test_processor();

        // Create original file
        let original_content = b"Hello, World! This is the original content.";
        let file_path = temp.path().join("test.txt");
        fs::write(&file_path, original_content)?;

        // Modified content
        let modified_content = b"Hello, Rust! This is the modified content.";

        // Generate signature from original
        let signatures = libsync3::generate_signatures(&original_content[..])
            .map_err(|e| anyhow!("Failed to generate signatures: {e}"))?;

        // Generate delta
        let delta = libsync3::generate_delta(&signatures, &modified_content[..])
            .map_err(|e| anyhow!("Failed to generate delta: {e}"))?;

        let expected_hash = blake3::hash(modified_content);
        let path = RelativePath::new("test.txt")?;
        let transfer_id = path_to_transfer_id(&path);

        // Start delta transfer
        processor.process_delta_sync(DeltaSyncOp::DeltaTransferStart {
            id: transfer_id,
            path: path.clone(),
            total_chunks: 1,
        })?;

        // Send delta chunk
        processor.process_delta_sync(DeltaSyncOp::DeltaTransferChunk {
            id: transfer_id,
            chunk_index: 0,
            chunk: delta,
        })?;

        // End transfer
        processor.process_delta_sync(DeltaSyncOp::DeltaTransferEnd {
            id: transfer_id,
            hash: expected_hash,
        })?;

        // Verify file content
        let result_content = fs::read(&file_path)?;
        assert_eq!(result_content, modified_content);

        Ok(())
    }

    #[test]
    fn test_delta_transfer_early_end() -> Result<()> {
        let (processor, temp) = create_test_processor();

        // Create original file
        let original_content = b"Original content here";
        let file_path = temp.path().join("early_end.txt");
        fs::write(&file_path, original_content)?;

        // Modified content
        let modified_content = b"Modified content here";

        // Generate signature and delta
        let signatures = libsync3::generate_signatures(&original_content[..])
            .map_err(|e| anyhow!("Failed to generate signatures: {e}"))?;
        let delta = libsync3::generate_delta(&signatures, &modified_content[..])
            .map_err(|e| anyhow!("Failed to generate delta: {e}"))?;

        let expected_hash = blake3::hash(modified_content);
        let path = RelativePath::new("early_end.txt")?;
        let transfer_id = path_to_transfer_id(&path);

        // Start delta transfer
        processor.process_delta_sync(DeltaSyncOp::DeltaTransferStart {
            id: transfer_id,
            path: path.clone(),
            total_chunks: 1,
        })?;

        // Send End BEFORE chunk (simulate race condition)
        processor.process_delta_sync(DeltaSyncOp::DeltaTransferEnd {
            id: transfer_id,
            hash: expected_hash,
        })?;

        // File should still have original content
        let content = fs::read(&file_path)?;
        assert_eq!(content, original_content);

        // Now send the chunk - should trigger completion
        processor.process_delta_sync(DeltaSyncOp::DeltaTransferChunk {
            id: transfer_id,
            chunk_index: 0,
            chunk: delta,
        })?;

        // File should now have modified content
        let content = fs::read(&file_path)?;
        assert_eq!(content, modified_content);

        Ok(())
    }

    #[test]
    fn test_delta_transfer_hash_mismatch() -> Result<()> {
        let (processor, temp) = create_test_processor();

        let original_content = b"Original";
        let file_path = temp.path().join("mismatch.txt");
        fs::write(&file_path, original_content)?;

        let modified_content = b"Modified";
        let signatures = libsync3::generate_signatures(&original_content[..])
            .map_err(|e| anyhow!("Failed to generate signatures: {e}"))?;
        let delta = libsync3::generate_delta(&signatures, &modified_content[..])
            .map_err(|e| anyhow!("Failed to generate delta: {e}"))?;

        // Use WRONG hash
        let wrong_hash = blake3::hash(b"Different content");
        let path = RelativePath::new("mismatch.txt")?;
        let transfer_id = path_to_transfer_id(&path);

        processor.process_delta_sync(DeltaSyncOp::DeltaTransferStart {
            id: transfer_id,
            path: path.clone(),
            total_chunks: 1,
        })?;

        processor.process_delta_sync(DeltaSyncOp::DeltaTransferChunk {
            id: transfer_id,
            chunk_index: 0,
            chunk: delta,
        })?;

        let result = processor.process_delta_sync(DeltaSyncOp::DeltaTransferEnd {
            id: transfer_id,
            hash: wrong_hash,
        });

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Hash mismatch"));

        // Original file should be unchanged
        let content = fs::read(&file_path)?;
        assert_eq!(content, original_content);

        Ok(())
    }

    #[test]
    fn test_delta_transfer_new_file() -> Result<()> {
        let (processor, temp) = create_test_processor();

        // No original file exists
        let new_content = b"Brand new file content";
        let path = RelativePath::new("new_file.txt")?;
        let file_path = temp.path().join("new_file.txt");

        // Generate delta from empty signatures
        let signatures = libsync3::Signatures::new(1024);
        let delta = libsync3::generate_delta(&signatures, &new_content[..])
            .map_err(|e| anyhow!("Failed to generate delta: {e}"))?;

        let expected_hash = blake3::hash(new_content);
        let transfer_id = path_to_transfer_id(&path);

        processor.process_delta_sync(DeltaSyncOp::DeltaTransferStart {
            id: transfer_id,
            path: path.clone(),
            total_chunks: 1,
        })?;

        processor.process_delta_sync(DeltaSyncOp::DeltaTransferChunk {
            id: transfer_id,
            chunk_index: 0,
            chunk: delta,
        })?;

        processor.process_delta_sync(DeltaSyncOp::DeltaTransferEnd {
            id: transfer_id,
            hash: expected_hash,
        })?;

        // File should now exist with new content
        assert!(file_path.exists());
        let content = fs::read(&file_path)?;
        assert_eq!(content, new_content);

        Ok(())
    }

    #[test]
    fn test_delta_transfer_unordered_chunks() -> Result<()> {
        let (processor, temp) = create_test_processor();

        // Create original file
        let original_content = b"AAAA BBBB CCCC DDDD";
        let file_path = temp.path().join("unordered.txt");
        fs::write(&file_path, original_content)?;

        // Modified content
        let modified_content = b"1111 2222 3333 4444";

        // Generate signature and delta
        let signatures = libsync3::generate_signatures(&original_content[..])
            .map_err(|e| anyhow!("Failed to generate signatures: {e}"))?;
        let delta = libsync3::generate_delta(&signatures, &modified_content[..])
            .map_err(|e| anyhow!("Failed to generate delta: {e}"))?;

        // Split delta into 3 chunks using drain to avoid Clone requirement
        let mut delta = delta;
        let total_len = delta.len();
        let chunk_size = (total_len + 2) / 3;

        let chunk0: Vec<_> = delta.drain(..chunk_size.min(delta.len())).collect();
        let chunk1: Vec<_> = delta.drain(..chunk_size.min(delta.len())).collect();
        let chunk2: Vec<_> = delta.drain(..).collect();

        let expected_hash = blake3::hash(modified_content);
        let path = RelativePath::new("unordered.txt")?;
        let transfer_id = path_to_transfer_id(&path);

        // Start transfer with 3 chunks
        processor.process_delta_sync(DeltaSyncOp::DeltaTransferStart {
            id: transfer_id,
            path: path.clone(),
            total_chunks: 3,
        })?;

        // Send chunks in REVERSE order (2, 1, 0)
        processor.process_delta_sync(DeltaSyncOp::DeltaTransferChunk {
            id: transfer_id,
            chunk_index: 2,
            chunk: chunk2,
        })?;

        processor.process_delta_sync(DeltaSyncOp::DeltaTransferChunk {
            id: transfer_id,
            chunk_index: 1,
            chunk: chunk1,
        })?;

        processor.process_delta_sync(DeltaSyncOp::DeltaTransferChunk {
            id: transfer_id,
            chunk_index: 0,
            chunk: chunk0,
        })?;

        // End transfer
        processor.process_delta_sync(DeltaSyncOp::DeltaTransferEnd {
            id: transfer_id,
            hash: expected_hash,
        })?;

        // Verify file content is correct
        let result_content = fs::read(&file_path)?;
        assert_eq!(result_content, modified_content);

        Ok(())
    }

    #[test]
    fn test_delta_transfer_unordered_chunks_with_early_end() -> Result<()> {
        let (processor, temp) = create_test_processor();

        // Create original file
        let original_content = b"Original file content for testing";
        let file_path = temp.path().join("unordered_early.txt");
        fs::write(&file_path, original_content)?;

        // Modified content
        let modified_content = b"Modified file content for testing";

        // Generate signature and delta
        let signatures = libsync3::generate_signatures(&original_content[..])
            .map_err(|e| anyhow!("Failed to generate signatures: {e}"))?;
        let delta = libsync3::generate_delta(&signatures, &modified_content[..])
            .map_err(|e| anyhow!("Failed to generate delta: {e}"))?;

        // Split delta into 4 chunks using drain
        let mut delta = delta;
        let total_len = delta.len();
        let chunk_size = (total_len + 3) / 4;

        let chunk0: Vec<_> = delta.drain(..chunk_size.min(delta.len())).collect();
        let chunk1: Vec<_> = delta.drain(..chunk_size.min(delta.len())).collect();
        let chunk2: Vec<_> = delta.drain(..chunk_size.min(delta.len())).collect();
        let chunk3: Vec<_> = delta.drain(..).collect();

        let expected_hash = blake3::hash(modified_content);
        let path = RelativePath::new("unordered_early.txt")?;
        let transfer_id = path_to_transfer_id(&path);

        // Start transfer
        processor.process_delta_sync(DeltaSyncOp::DeltaTransferStart {
            id: transfer_id,
            path: path.clone(),
            total_chunks: 4,
        })?;

        // Send chunk 3 first
        processor.process_delta_sync(DeltaSyncOp::DeltaTransferChunk {
            id: transfer_id,
            chunk_index: 3,
            chunk: chunk3,
        })?;

        // Send End BEFORE all chunks arrive
        processor.process_delta_sync(DeltaSyncOp::DeltaTransferEnd {
            id: transfer_id,
            hash: expected_hash,
        })?;

        // File should still have original content
        let content = fs::read(&file_path)?;
        assert_eq!(content, original_content);

        // Send chunk 0
        processor.process_delta_sync(DeltaSyncOp::DeltaTransferChunk {
            id: transfer_id,
            chunk_index: 0,
            chunk: chunk0,
        })?;

        // Send chunk 2
        processor.process_delta_sync(DeltaSyncOp::DeltaTransferChunk {
            id: transfer_id,
            chunk_index: 2,
            chunk: chunk2,
        })?;

        // File should still have original content (chunk 1 missing)
        let content = fs::read(&file_path)?;
        assert_eq!(content, original_content);

        // Send final chunk 1 - should trigger completion
        processor.process_delta_sync(DeltaSyncOp::DeltaTransferChunk {
            id: transfer_id,
            chunk_index: 1,
            chunk: chunk1,
        })?;

        // File should now have modified content
        let content = fs::read(&file_path)?;
        assert_eq!(content, modified_content);

        Ok(())
    }

    #[test]
    fn test_delta_transfer_duplicate_chunk_rejected() -> Result<()> {
        let (processor, temp) = create_test_processor();

        let original_content = b"Test content";
        let file_path = temp.path().join("duplicate.txt");
        fs::write(&file_path, original_content)?;

        let path = RelativePath::new("duplicate.txt")?;
        let transfer_id = path_to_transfer_id(&path);

        processor.process_delta_sync(DeltaSyncOp::DeltaTransferStart {
            id: transfer_id,
            path: path.clone(),
            total_chunks: 2,
        })?;

        // Send chunk 0
        processor.process_delta_sync(DeltaSyncOp::DeltaTransferChunk {
            id: transfer_id,
            chunk_index: 0,
            chunk: vec![],
        })?;

        // Try to send chunk 0 again - should fail
        let result = processor.process_delta_sync(DeltaSyncOp::DeltaTransferChunk {
            id: transfer_id,
            chunk_index: 0,
            chunk: vec![],
        });

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Duplicate chunk"));

        Ok(())
    }

    #[test]
    fn test_delta_transfer_chunk_index_out_of_bounds() -> Result<()> {
        let (processor, temp) = create_test_processor();

        let original_content = b"Test";
        let file_path = temp.path().join("bounds.txt");
        fs::write(&file_path, original_content)?;

        let path = RelativePath::new("bounds.txt")?;
        let transfer_id = path_to_transfer_id(&path);

        processor.process_delta_sync(DeltaSyncOp::DeltaTransferStart {
            id: transfer_id,
            path: path.clone(),
            total_chunks: 2,
        })?;

        // Try to send chunk with index 5 when total_chunks is 2
        let result = processor.process_delta_sync(DeltaSyncOp::DeltaTransferChunk {
            id: transfer_id,
            chunk_index: 5,
            chunk: vec![],
        });

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("out of bounds"));

        Ok(())
    }
}
