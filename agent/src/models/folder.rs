use super::{FileMetadata, RelativePath};
use crate::protocol::{ChunkedTransferOp, FileOperation, FolderId};
use anyhow::{Context, Result, bail};
use blake3::Hash;
use serde::{Deserialize, Serialize};
use std::io::{Seek, Write};
use std::{fs, path::PathBuf};
use time::OffsetDateTime;

#[derive(Debug, Serialize, Deserialize)]
pub struct Folder {
    id: FolderId,
    name: String,
    path: PathBuf,
    last_successful_sync: OffsetDateTime,
}

impl Folder {
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
            ChunkedTransferOp::Start { id, total_size } => self.handle_start(id, total_size),

            ChunkedTransferOp::Chunk {
                id,
                index,
                chunk_size,
                data,
            } => self.handle_chunk(id, index, chunk_size, &data),

            ChunkedTransferOp::End {
                id,
                path,
                hash,
                metadata,
            } => self.handle_end(id, &self.resolve(&path), hash, &metadata),

            ChunkedTransferOp::Abort { id, reason } => {
                println!("TODO: replace this println! Abort: {reason}");
                let _ = fs::remove_file(self.temp_path_ref(id));
                Ok(())
            }
        }
    }

    fn handle_start(&self, id: u64, total_size: u64) -> Result<()> {
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

        Ok(())
    }

    fn handle_chunk(&self, id: u64, index: u32, chunk_size: u32, data: &[u8]) -> Result<()> {
        let temp_path = self.temp_path_ref(id);

        // Write chunk at correct offset
        let offset = u64::from(index) * u64::from(chunk_size);
        let mut file = fs::OpenOptions::new().write(true).open(&temp_path)?;
        file.seek(std::io::SeekFrom::Start(offset))?;
        file.write_all(data)?;
        file.sync_data()?;

        Ok(())
    }

    fn handle_end(
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
            let _ = fs::remove_file(temp_path);
            bail!("Hash mismatch");
        }

        // Atomic move to final location
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&temp_path, path)?;

        metadata.apply_to(path)?;

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
