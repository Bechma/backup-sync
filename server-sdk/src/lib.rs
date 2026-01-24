use serde::{Deserialize, Serialize};

pub type UserId = String;
pub type ComputerId = String;
pub type FolderId = String;

/// A computer registered by a user
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Computer {
    pub id: ComputerId,
    pub name: String,
    pub online: bool,
}

/// A sync folder with an origin and multiple backups
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncFolder {
    pub id: FolderId,
    pub name: String,
    /// The computer currently acting as origin (source of truth)
    pub origin_computer: ComputerId,
    /// Computers that have a backup copy of this folder
    pub backup_computers: Vec<ComputerId>,
    /// Whether all backups are in sync with origin (no pending operations)
    pub is_synced: bool,
    /// Number of pending operations waiting to be applied
    pub pending_operations: u64,
}

/// User with their computers and sync folders
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,
    pub name: String,
    pub computers: Vec<Computer>,
    pub sync_folders: Vec<SyncFolder>,
}
