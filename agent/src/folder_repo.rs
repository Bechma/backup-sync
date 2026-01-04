use std::collections::HashMap;

use crate::models::Folder;
use crate::protocol::{FolderId, FolderOperation};
use anyhow::{Context, Result};

pub struct FolderRepo {
    folders: HashMap<FolderId, Folder>,
}

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

    pub fn process_operation(&mut self, operation: FolderOperation) -> Result<()> {
        match operation {
            FolderOperation::Init { folder_id: _ } => {
                todo!("Not implemented")
            }
            FolderOperation::Operation {
                folder_id,
                operation,
                operation_id: _,
            } => self
                .folders
                .get(&folder_id)
                .context("Folder not found")?
                .process_operation(operation),
            FolderOperation::RequestSync { folder_id: _ } => {
                todo!("Not implemented")
            }
        }
    }
}
