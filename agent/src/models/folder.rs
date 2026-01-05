use super::{FileMetadata, RelativePath};
use crate::protocol::{ChunkedTransferOp, FileOperation, FolderId};
use anyhow::{Context, Result, bail};
use blake3::Hash;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{Seek, Write};
use std::sync::{Arc, Mutex};
use std::{fs, path::PathBuf};
use time::OffsetDateTime;

#[derive(Debug, Clone)]
struct TransferState {
    total_chunks: u64,
    chunk_size: u64,
    received_chunks: HashSet<u64>,
    pending_end: Option<PendingEnd>,
}

#[derive(Debug, Clone)]
struct PendingEnd {
    path: PathBuf,
    hash: Hash,
    metadata: FileMetadata,
}

fn default_transfer_states() -> Arc<Mutex<HashMap<u64, TransferState>>> {
    Arc::new(Mutex::new(HashMap::new()))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Folder {
    id: FolderId,
    name: String,
    path: PathBuf,
    last_successful_sync: OffsetDateTime,
    #[serde(skip, default = "default_transfer_states")]
    transfer_states: Arc<Mutex<HashMap<u64, TransferState>>>,
}

impl Folder {
    fn lock_transfer_states(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, HashMap<u64, TransferState>>> {
        self.transfer_states
            .lock()
            .map_err(|e| anyhow::anyhow!("Transfer states mutex poisoned: {}", e))
    }

    fn resolve(&self, path: &RelativePath) -> PathBuf {
        path.resolve(&self.path)
    }

    fn process_delete(&self, path: &RelativePath) -> Result<()> {
        let resolved_path = self.resolve(path);
        if !resolved_path.exists() {
            return Ok(());
        }
        if resolved_path.is_dir() {
            fs::remove_dir_all(&resolved_path).context("Failed to remove directory recursevely")?;
        } else {
            fs::remove_file(&resolved_path).context("Failed to remove file")?;
        }
        Ok(())
    }

    fn process_create_dir(&self, path: &RelativePath) -> Result<()> {
        let resolved_path = self.resolve(path);
        fs::create_dir(&resolved_path).context("Failed to create directory")
    }

    fn process_rename(&self, from: &RelativePath, to: &RelativePath) -> Result<()> {
        let from_path = self.resolve(from);
        let mut to_path = self.resolve(to);
        if !from_path.exists() {
            bail!("File not found: {}", from_path.display());
        }

        if to_path.exists() {
            let filename = format!(
                "{}_{}_conflict",
                to_path
                    .file_name()
                    .and_then(|x| x.to_str())
                    .context("Failed to get file name")?,
                time::OffsetDateTime::now_utc(),
            );
            to_path.set_file_name(filename);
        }

        fs::rename(&from_path, &to_path).context("Failed to rename file")
    }

    fn process_write_file(
        &self,
        path: &RelativePath,
        content: Vec<u8>,
        metadata: &FileMetadata,
        hash: Hash,
    ) -> Result<()> {
        let resolved_path = self.resolve(path);

        let computed_hash = blake3::hash(&content);

        if computed_hash != hash {
            bail!("Hash mismatch: expected {hash}, got {computed_hash}");
        }

        fs::write(&resolved_path, content).with_context(|| {
            format!(
                "Problems while writing the file: {}",
                resolved_path.display()
            )
        })?;

        metadata
            .apply_to(&resolved_path)
            .context("Failed to apply metadata")
    }

    pub fn process_operation(&self, operation: FileOperation) -> Result<()> {
        match operation {
            FileOperation::Delete { path } => self.process_delete(&path),
            FileOperation::CreateDir { path } => self.process_create_dir(&path),
            FileOperation::Rename { from, to } => self.process_rename(&from, &to),
            FileOperation::WriteFile {
                path,
                content,
                metadata,
                hash,
            } => self.process_write_file(&path, content, &metadata, hash),
            FileOperation::ChunkedTransfer(chunked_transfer_op) => {
                self.process_chunked_transfer(chunked_transfer_op)
            }
            FileOperation::DeltaSync(_delta_sync_op) => todo!("not implemented yet"),
        }
    }

    fn temp_folder_path(&self) -> PathBuf {
        const TEMP_DIR_REF: &str = "backup_sync_temp_dir";
        std::env::temp_dir()
            .join(TEMP_DIR_REF)
            .join(self.id.to_string())
    }

    fn temp_path_ref(&self, id: u64) -> PathBuf {
        self.temp_folder_path().join(format!("{id}.tmp"))
    }

    fn process_chunked_transfer(&self, op: ChunkedTransferOp) -> Result<()> {
        match op {
            ChunkedTransferOp::Start {
                id,
                total_size,
                chunk_size,
            } => self.handle_start(id, total_size, chunk_size),

            ChunkedTransferOp::Chunk { id, index, data } => self.handle_chunk(id, index, &data),

            ChunkedTransferOp::End {
                id,
                path,
                hash,
                metadata,
            } => self.handle_end(id, &self.resolve(&path), hash, &metadata),

            ChunkedTransferOp::Abort { id, reason } => {
                println!("TODO: replace this println! Abort: {reason}");
                let _ = fs::remove_file(self.temp_path_ref(id));
                // Clean up transfer state
                self.lock_transfer_states()?.remove(&id);
                Ok(())
            }
        }
    }

    fn handle_start(&self, id: u64, total_size: u64, chunk_size: u64) -> Result<()> {
        // Pre-allocate temp file
        let temp_path = self.temp_path_ref(id);
        if let Some(parent) = temp_path.parent() {
            fs::create_dir_all(parent)?;
        }
        if temp_path.exists() {
            // This means we start over
            fs::remove_file(&temp_path)
                .with_context(|| format!("Cannot remove temp file: {}", temp_path.display()))?;
        }
        let file = fs::File::create(&temp_path)
            .with_context(|| format!("Cannot create temp file: {}", temp_path.display()))?;
        file.set_len(total_size)
            .with_context(|| format!("Failed to set file length: {}", temp_path.display()))?;

        // Initialize transfer state
        let total_chunks = total_size.div_ceil(chunk_size);
        let mut states = self.lock_transfer_states()?;
        states.insert(
            id,
            TransferState {
                total_chunks,
                chunk_size,
                received_chunks: HashSet::new(),
                pending_end: None,
            },
        );

        Ok(())
    }

    fn handle_chunk(&self, id: u64, index: u64, data: &[u8]) -> Result<()> {
        let temp_path = self.temp_path_ref(id);

        // Get chunk_size from state
        let chunk_size = {
            let states = self.lock_transfer_states()?;
            states
                .get(&id)
                .map(|s| s.chunk_size)
                .context("Transfer not started")?
        };

        // Write chunk at correct offset
        let offset = index * chunk_size;
        let mut file = fs::OpenOptions::new().write(true).open(&temp_path)?;
        file.seek(std::io::SeekFrom::Start(offset))?;
        file.write_all(data)?;
        file.sync_data()?;

        // Mark chunk as received
        let mut states = self.lock_transfer_states()?;
        if let Some(state) = states.get_mut(&id) {
            state.received_chunks.insert(index);

            // Check if we have pending end and all chunks are now received
            if let Some(pending_end) = state.pending_end.clone()
                && state.received_chunks.len() as u64 == state.total_chunks
            {
                // All chunks received, process the pending end
                drop(states); // Release lock before calling handle_end_internal
                return self.handle_end_internal(
                    id,
                    &pending_end.path,
                    pending_end.hash,
                    &pending_end.metadata,
                );
            }
        }

        Ok(())
    }

    fn handle_end(
        &self,
        id: u64,
        path: &PathBuf,
        expected_hash: Hash,
        metadata: &FileMetadata,
    ) -> Result<()> {
        // Check if all chunks have been received
        let all_chunks_received = {
            let mut states = self.lock_transfer_states()?;
            if let Some(state) = states.get_mut(&id) {
                let all_received = state.received_chunks.len() as u64 == state.total_chunks;
                if !all_received {
                    // Store pending end for later processing
                    state.pending_end = Some(PendingEnd {
                        path: path.clone(),
                        hash: expected_hash,
                        metadata: metadata.clone(),
                    });
                }
                all_received
            } else {
                bail!("Transfer state not found for id: {id}");
            }
        };

        if all_chunks_received {
            self.handle_end_internal(id, path, expected_hash, metadata)
        } else {
            // End message arrived before all chunks, will be processed when last chunk arrives
            Ok(())
        }
    }

    fn handle_end_internal(
        &self,
        id: u64,
        path: &PathBuf,
        expected_hash: Hash,
        metadata: &FileMetadata,
    ) -> Result<()> {
        // Verify hash
        let temp_path = self.temp_path_ref(id);
        let actual_hash = hash_file(&temp_path)?;
        if actual_hash != expected_hash {
            let _ = fs::remove_file(&temp_path);
            // Clean up transfer state
            self.lock_transfer_states()?.remove(&id);
            bail!("Hash mismatch: expected {expected_hash}, got {actual_hash}");
        }

        // Atomic move to final location
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&temp_path, path)?;

        metadata.apply_to(path)?;

        // Clean up transfer state
        self.lock_transfer_states()?.remove(&id);

        Ok(())
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        // TODO: When added resumability support, we should not remove temp files
        let _ = fs::remove_dir_all(self.temp_folder_path());
    }
}

fn hash_file(reference: &PathBuf) -> Result<Hash> {
    let mut hasher = blake3::Hasher::new();
    let file = fs::File::open(reference).context("Failed to open file")?;
    hasher
        .update_reader(file)
        .map(|x| x.finalize())
        .with_context(|| {
            format!(
                "Failed to update hasher with reader: {}",
                reference.display()
            )
        })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::file_metadata::{FileType, Permissions};
    use crate::protocol::ChunkedTransferOp;
    use tempfile::tempdir;

    fn create_test_folder() -> Folder {
        let temp_dir = tempdir().unwrap();
        Folder {
            id: uuid::Uuid::new_v4(),
            name: "test_folder".to_string(),
            path: temp_dir.path().to_path_buf(),
            last_successful_sync: OffsetDateTime::now_utc(),
            transfer_states: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn create_dummy_metadata() -> FileMetadata {
        FileMetadata::new(
            FileType::File,
            0,
            Permissions::default_file(),
            OffsetDateTime::now_utc(),
        )
    }

    #[test]
    fn test_chunked_transfer_linear() -> Result<()> {
        let folder = create_test_folder();
        let transfer_id = 1;
        let file_content = b"Hello, World!";
        let chunk_size = 5;

        // Split content into chunks: "Hello", ", Wor", "ld!"
        let chunks: Vec<&[u8]> = file_content.chunks(chunk_size).collect();
        let total_size = file_content.len() as u64;
        let hash = blake3::hash(file_content);
        let target_path = RelativePath::new("test.txt")?;

        // 1. Start transfer
        folder.process_chunked_transfer(ChunkedTransferOp::Start {
            id: transfer_id,
            total_size,
            chunk_size: chunk_size as u64,
        })?;

        // 2. Send chunks in order
        for (i, chunk) in chunks.iter().enumerate() {
            folder.process_chunked_transfer(ChunkedTransferOp::Chunk {
                id: transfer_id,
                index: i as u64,
                data: chunk.to_vec(),
            })?;
        }

        // 3. End transfer
        folder.process_chunked_transfer(ChunkedTransferOp::End {
            id: transfer_id,
            path: target_path.clone(),
            hash,
            metadata: create_dummy_metadata(),
        })?;

        // Verify file exists and content matches
        let file_path = folder.resolve(&target_path);
        assert!(file_path.exists());
        let content = fs::read(&file_path)?;
        assert_eq!(content, file_content);

        Ok(())
    }

    #[test]
    fn test_chunked_transfer_unordered() -> Result<()> {
        let folder = create_test_folder();
        let transfer_id = 2;
        let file_content = b"Unordered chunks test";
        let chunk_size = 5;

        let chunks: Vec<&[u8]> = file_content.chunks(chunk_size).collect();
        let total_size = file_content.len() as u64;
        let hash = blake3::hash(file_content);
        let target_path = RelativePath::new("unordered.txt")?;

        folder.process_chunked_transfer(ChunkedTransferOp::Start {
            id: transfer_id,
            total_size,
            chunk_size: chunk_size as u64,
        })?;

        // Send chunks in reverse order
        for i in (0..chunks.len()).rev() {
            folder.process_chunked_transfer(ChunkedTransferOp::Chunk {
                id: transfer_id,
                index: i as u64,
                data: chunks[i].to_vec(),
            })?;
        }

        folder.process_chunked_transfer(ChunkedTransferOp::End {
            id: transfer_id,
            path: target_path.clone(),
            hash,
            metadata: create_dummy_metadata(),
        })?;

        let file_path = folder.resolve(&target_path);
        assert!(file_path.exists());
        let content = fs::read(&file_path)?;
        assert_eq!(content, file_content);

        Ok(())
    }

    #[test]
    fn test_chunked_transfer_early_end() -> Result<()> {
        let folder = create_test_folder();
        let transfer_id = 3;
        let file_content = b"Race condition test";
        let chunk_size = 5;

        let chunks: Vec<&[u8]> = file_content.chunks(chunk_size).collect();
        let total_size = file_content.len() as u64;
        let hash = blake3::hash(file_content);
        let target_path = RelativePath::new("race.txt")?;

        folder.process_chunked_transfer(ChunkedTransferOp::Start {
            id: transfer_id,
            total_size,
            chunk_size: chunk_size as u64,
        })?;

        // Send first chunk
        folder.process_chunked_transfer(ChunkedTransferOp::Chunk {
            id: transfer_id,
            index: 0,
            data: chunks[0].to_vec(),
        })?;

        // Send End message BEFORE other chunks (simulate race condition)
        folder.process_chunked_transfer(ChunkedTransferOp::End {
            id: transfer_id,
            path: target_path.clone(),
            hash,
            metadata: create_dummy_metadata(),
        })?;

        // File should NOT exist yet
        let file_path = folder.resolve(&target_path);
        assert!(!file_path.exists());

        // Send remaining chunks
        for i in 1..chunks.len() {
            folder.process_chunked_transfer(ChunkedTransferOp::Chunk {
                id: transfer_id,
                index: i as u64,
                data: chunks[i].to_vec(),
            })?;
        }

        // File SHOULD exist now
        assert!(file_path.exists());
        let content = fs::read(&file_path)?;
        assert_eq!(content, file_content);

        Ok(())
    }

    #[test]
    fn test_chunked_transfer_hash_mismatch() -> Result<()> {
        let folder = create_test_folder();
        let transfer_id = 4;
        let file_content = b"Corrupted content";
        let chunk_size = 5;
        let total_size = file_content.len() as u64;

        // Use WRONG hash
        let hash = blake3::hash(b"Different content");
        let target_path = RelativePath::new("corrupt.txt")?;

        folder.process_chunked_transfer(ChunkedTransferOp::Start {
            id: transfer_id,
            total_size,
            chunk_size: chunk_size as u64,
        })?;

        let chunks: Vec<&[u8]> = file_content.chunks(chunk_size).collect();
        for (i, chunk) in chunks.iter().enumerate() {
            folder.process_chunked_transfer(ChunkedTransferOp::Chunk {
                id: transfer_id,
                index: i as u64,
                data: chunk.to_vec(),
            })?;
        }

        // Expect error on End
        let result = folder.process_chunked_transfer(ChunkedTransferOp::End {
            id: transfer_id,
            path: target_path.clone(),
            hash,
            metadata: create_dummy_metadata(),
        });

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Hash mismatch"));

        // File should not exist
        assert!(!folder.resolve(&target_path).exists());

        Ok(())
    }
}
