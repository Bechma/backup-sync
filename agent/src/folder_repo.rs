use std::collections::HashMap;

use crate::models::Folder;
use crate::protocol::{FolderId, FolderOperation, FolderResponse, SyncManifest};
use anyhow::{Context, Result};

pub struct FolderRepo {
    folders: HashMap<FolderId, Folder>,
}

const DEFAULT_CHUNK_SIZE: u64 = 1024 * 1024; // 1MB chunks

impl Default for FolderRepo {
    fn default() -> Self {
        Self::new()
    }
}

impl FolderRepo {
    #[must_use]
    pub fn new() -> Self {
        Self {
            folders: HashMap::new(),
        }
    }

    pub fn insert(&mut self, folder_id: FolderId, folder: Folder) -> Option<Folder> {
        self.folders.insert(folder_id, folder)
    }

    pub fn remove(&mut self, folder_id: &FolderId) -> Option<Folder> {
        self.folders.remove(folder_id)
    }

    pub fn process_operation(
        &mut self,
        operation: FolderOperation,
    ) -> Result<Option<FolderResponse>> {
        match operation {
            FolderOperation::Init { folder_id: _ } => {
                todo!("Not implemented")
            }
            FolderOperation::Operation {
                folder_id,
                operation,
                operation_id: _,
            } => {
                self.folders
                    .get(&folder_id)
                    .context("Folder not found")?
                    .process_operation(operation)?;
                Ok(None)
            }
            FolderOperation::RequestSync { folder_id } => {
                let manifest = self.generate_manifest(&folder_id)?;
                Ok(Some(FolderResponse::SyncManifest(manifest)))
            }
        }
    }

    pub fn generate_manifest(&self, folder_id: &FolderId) -> Result<SyncManifest> {
        self.generate_manifest_with_chunk_size(folder_id, DEFAULT_CHUNK_SIZE)
    }

    pub fn generate_manifest_with_chunk_size(
        &self,
        folder_id: &FolderId,
        chunk_size: u64,
    ) -> Result<SyncManifest> {
        let folder = self.folders.get(folder_id).context("Folder not found")?;
        folder.generate_manifest(chunk_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RelativePath;
    use std::fs;
    use std::io::Write;
    use tempfile::tempdir;

    fn create_test_folder() -> (Folder, tempfile::TempDir) {
        let temp_dir = tempdir().unwrap();
        let folder = Folder::new(
            uuid::Uuid::new_v4(),
            "test_folder".to_string(),
            temp_dir.path().to_path_buf(),
        );
        (folder, temp_dir)
    }

    #[test]
    fn test_request_sync_returns_manifest() -> Result<()> {
        let (folder, _temp_dir) = create_test_folder();
        let folder_id = *folder.id();

        // Create a file in the folder
        let file_path = folder.path().join("test.txt");
        let mut file = fs::File::create(&file_path)?;
        file.write_all(b"Hello Manifest")?;

        let mut repo = FolderRepo::new();
        repo.insert(folder_id, folder);

        let op = FolderOperation::RequestSync { folder_id };
        let result = repo.process_operation(op)?;

        if let Some(FolderResponse::SyncManifest(manifest)) = result {
            assert_eq!(manifest.folder_id, folder_id);
            assert_eq!(manifest.file_count, 1);
            assert_eq!(manifest.total_size, 14);

            let relative_path = RelativePath::new("test.txt")?;
            assert!(manifest.files.contains_key(&relative_path));

            let entry = manifest.files.get(&relative_path).unwrap();
            assert_eq!(entry.metadata.size(), 14);
        } else {
            panic!("Expected SyncManifest operation");
        }

        Ok(())
    }
}
