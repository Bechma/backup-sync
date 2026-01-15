use super::{FileMetadata, RelativePath};
use crate::buffers::{DeltaSyncProcessor, FileChunkProcessor};
use crate::protocol::{FileEntry, FileOperation, FolderId, SyncManifest};
use anyhow::{bail, Context, Result};
use blake3::Hash;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Seek, Write};
use std::{fs, path::PathBuf};
use time::OffsetDateTime;

#[derive(Debug)]
pub struct Folder {
    id: FolderId,
    name: String,
    path: PathBuf,
    file_chunk_processor: FileChunkProcessor,
    delta_sync_processor: DeltaSyncProcessor,
}

impl Folder {
    #[must_use]
    pub fn new(id: FolderId, name: &str, path: PathBuf) -> Self {
        Self {
            id,
            name: name.to_owned(),
            path: path.clone(),
            file_chunk_processor: FileChunkProcessor::new(id.to_string(), path.clone()),
            delta_sync_processor: DeltaSyncProcessor::new(id.to_string(), path),
        }
    }

    #[must_use]
    pub fn id(&self) -> &FolderId {
        &self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn generate_manifest(&self, chunk_size: u64) -> Result<SyncManifest> {
        let mut files = HashMap::new();
        let mut total_size = 0u64;
        let mut file_count = 0u64;

        self.walk_directory(
            &self.path,
            &mut files,
            &mut total_size,
            &mut file_count,
            chunk_size,
        )
        .context("Failed to walk directory")?;

        Ok(SyncManifest {
            folder_id: self.id,
            version: 1,
            timestamp: OffsetDateTime::now_utc().unix_timestamp(),
            files,
            total_size,
            file_count,
        })
    }

    fn walk_directory(
        &self,
        dir: &PathBuf,
        files: &mut HashMap<RelativePath, FileEntry>,
        total_size: &mut u64,
        file_count: &mut u64,
        chunk_size: u64,
    ) -> Result<()> {
        if !dir.exists() {
            return Ok(());
        }

        for entry in fs::read_dir(dir).context("Failed to read directory")? {
            let entry = entry.context("Failed to read directory entry")?;
            let path = entry.path();
            let metadata = entry.metadata().context("Failed to read metadata")?;

            if metadata.is_dir() {
                self.walk_directory(&path, files, total_size, file_count, chunk_size)?;
            } else if metadata.is_file() {
                let relative_path = path
                    .strip_prefix(&self.path)
                    .context("Failed to strip prefix")?
                    .to_str()
                    .context("Failed to convert path to string")?;
                let relative_path = RelativePath::new(relative_path)?;

                let file_size = metadata.len();
                let file = fs::File::open(&path).context("Failed to open file")?;

                let mut hashing_reader = crate::Hashing::new(file);

                let signature = libsync3::generate_signatures_with_block_size(
                    &mut hashing_reader,
                    chunk_size as usize,
                )
                .map_err(|e| anyhow::anyhow!("Failed to generate signature: {e}"))?;

                let hash = hashing_reader.finalize();

                files.insert(
                    relative_path,
                    FileEntry {
                        hash,
                        metadata: FileMetadata::from_std_metadata(&metadata, &path)?,
                        signature,
                    },
                );

                *total_size += file_size;
                *file_count += 1;
            }
        }

        Ok(())
    }

    fn process_delete(&self, path: &RelativePath) -> Result<()> {
        let resolved_path = path.resolve(&self.path);
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
        let resolved_path = path.resolve(&self.path);
        fs::create_dir(&resolved_path).context("Failed to create directory")
    }

    fn process_rename(&self, from: &RelativePath, to: &RelativePath) -> Result<()> {
        let from_path = from.resolve(&self.path);
        let mut to_path = to.resolve(&self.path);
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
        let resolved_path = path.resolve(&self.path);

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
            FileOperation::ChunkedTransfer(chunked_transfer_op) => self
                .file_chunk_processor
                .process_chunked_transfer(chunked_transfer_op),
            FileOperation::DeltaSync(delta_sync_op) => self
                .delta_sync_processor
                .process_delta_sync(delta_sync_op)
                .map(|_| ()),
        }
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        // TODO: When added resumability support, we should not remove temp files
        _ = fs::remove_dir_all(crate::buffers::temp_folder_path(self.id.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::file_metadata::{FileType, Permissions};
    use tempfile::tempdir;

    fn create_test_folder() -> (Folder, tempfile::TempDir) {
        let temp_dir = tempdir().unwrap();
        let folder = Folder::new(
            uuid::Uuid::new_v4(),
            "test_folder",
            temp_dir.path().to_path_buf(),
        );
        (folder, temp_dir)
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
    fn test_generate_manifest_empty_folder() -> Result<()> {
        let (folder, _temp) = create_test_folder();
        let manifest = folder.generate_manifest(1024)?;

        assert_eq!(manifest.files.len(), 0);
        assert_eq!(manifest.total_size, 0);
        assert_eq!(manifest.file_count, 0);
        Ok(())
    }

    #[test]
    fn test_generate_manifest_single_file() -> Result<()> {
        let (folder, _temp) = create_test_folder();
        let file_path = folder.path.join("test.txt");
        fs::write(&file_path, "Hello World")?;

        let manifest = folder.generate_manifest(1024)?;

        assert_eq!(manifest.files.len(), 1);
        assert_eq!(manifest.total_size, 11);
        assert_eq!(manifest.file_count, 1);

        let relative_path = RelativePath::new("test.txt")?;
        let entry = manifest.files.get(&relative_path).unwrap();
        assert_eq!(entry.metadata.size(), 11);
        assert_eq!(entry.hash, crate::buffers::hash_file(&file_path)?);
        assert_eq!(entry.signature.len(), 1);
        Ok(())
    }

    #[test]
    fn test_generate_manifest_nested_files() -> Result<()> {
        let (folder, _temp) = create_test_folder();
        let sub_dir = folder.path.join("sub");
        fs::create_dir(&sub_dir)?;
        fs::write(sub_dir.join("nested.txt"), "Nested Content")?;
        fs::write(folder.path.join("root.txt"), "Root Content")?;

        let manifest = folder.generate_manifest(1024)?;

        assert_eq!(manifest.files.len(), 2);
        assert_eq!(manifest.file_count, 2);

        let nested_path = RelativePath::new("sub/nested.txt")?;
        assert!(manifest.files.contains_key(&nested_path));

        let root_path = RelativePath::new("root.txt")?;
        assert!(manifest.files.contains_key(&root_path));
        Ok(())
    }

    #[test]
    fn test_generate_manifest_large_file_chunking() {
        let (folder, _temp) = create_test_folder();
        let file_path = folder.path.join("large.bin");
        let content = [vec![0u8; 1024], vec![1u8; 1024]].concat(); // 2KB
        fs::write(&file_path, &content).unwrap();

        // Chunk size 1KB, should produce 2 chunks
        let manifest = folder.generate_manifest(1024).unwrap();

        let relative_path = RelativePath::new("large.bin").unwrap();
        let entry = manifest.files.get(&relative_path).unwrap();

        assert_eq!(entry.metadata.size(), 2048);
        assert_eq!(entry.signature.block_size(), 1024);

        let chunks = &entry.signature;
        assert_eq!(chunks.len(), 2);

        let roll = libsync3::rolling::RollingChecksum::compute(&content[..1024]);
        let strong = chunks.weak(roll).unwrap();
        assert_eq!(strong.len(), 1);
        assert_eq!(strong[0].block_index, 0);
        assert_eq!(strong[0].strong, libsync3::xxh3_128(&content[..1024]));

        let roll = libsync3::rolling::RollingChecksum::compute(&content[1024..]);
        let strong = chunks.weak(roll).unwrap();
        assert_eq!(strong.len(), 1);
        assert_eq!(strong[0].block_index, 1);
        assert_eq!(strong[0].strong, libsync3::xxh3_128(&content[1024..]));
    }
}
