use super::relative_path::RelativePath;
use crate::models::FileMetadata;
use crate::protocol::{FileOperation, FolderId};
use anyhow::{Context, Result};
use blake3::Hash;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use time::OffsetDateTime;

#[derive(Debug, Serialize, Deserialize)]
pub struct Folder {
    id: FolderId,
    name: String,
    path: PathBuf,
    last_successful_sync: OffsetDateTime,
    pending_operations: u64,
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
            std::fs::remove_dir_all(&resolved_path)
                .context("Failed to remove directory recursevely")?;
        } else {
            std::fs::remove_file(&resolved_path).context("Failed to remove file")?;
        }
        Ok(())
    }

    fn process_create_dir(&self, path: &RelativePath) -> Result<()> {
        let resolved_path = self.resolve(path);
        std::fs::create_dir(&resolved_path).context("Failed to create directory")
    }

    fn process_rename(&self, from: &RelativePath, to: &RelativePath) -> Result<()> {
        let from_path = self.resolve(from);
        let mut to_path = self.resolve(to);
        if !from_path.exists() {
            return Err(anyhow::anyhow!("File not found: {}", from_path.display()));
        }

        if to_path.exists() {
            let filename = format!(
                "{}_{}",
                to_path
                    .file_name()
                    .map(|x| x.to_str())
                    .flatten()
                    .context("Failed to get file name")?,
                time::OffsetDateTime::now_utc(),
            );
            to_path.set_file_name(filename);
        }

        std::fs::rename(&from_path, &to_path).context("Failed to rename file")
    }

    fn process_write_file(
        &self,
        path: &RelativePath,
        content: Vec<u8>,
        metadata: FileMetadata,
        hash: Hash,
    ) -> Result<()> {
        let resolved_path = self.resolve(path);

        let computed_hash = blake3::hash(&content);

        if computed_hash != hash {
            return Err(anyhow::anyhow!("Hash mismatch"));
        }

        std::fs::write(&resolved_path, content).with_context(|| {
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
            } => self.process_write_file(&path, content, metadata, hash),
            FileOperation::ChunkedTransfer(_chunked_transfer_op) => todo!("not implemented yet"),
            FileOperation::DeltaSync(_delta_sync_op) => todo!("not implemented yet"),
        }
    }
}
