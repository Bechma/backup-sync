use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileOperation {
    /// Create a new file with content
    CreateFile {
        relative_path: PathBuf,
        content: Vec<u8>,
    },
    /// Create a directory
    CreateDir { relative_path: PathBuf },
    /// Delete a file
    RemoveFile { relative_path: PathBuf },
    /// Delete a directory recursively
    RemoveDir { relative_path: PathBuf },
    /// Rename/move a file
    RenameFile {
        from_relative: PathBuf,
        to_relative: PathBuf,
    },
    /// Apply delta to modify a file (rsync-style)
    ApplyDelta {
        relative_path: PathBuf,
        delta: Vec<u8>,
    },
    /// Request signature for a file (for delta calculation)
    RequestSignature { relative_path: PathBuf },
    /// Response with file signature
    SignatureResponse {
        relative_path: PathBuf,
        signature: Vec<u8>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    /// Authenticate as a user on a specific computer
    Authenticate {
        user_id: UserId,
        computer_id: ComputerId,
    },
    /// Register a new computer for this user
    RegisterComputer { name: String },
    /// Create a new sync folder with this computer as origin
    CreateSyncFolder { name: String },
    /// Add this computer as a backup for a sync folder
    JoinSyncFolder { folder_id: FolderId },
    /// Leave a sync folder (remove this computer from backups)
    LeaveSyncFolder { folder_id: FolderId },
    /// Request to become the new origin (only allowed when folder is synced)
    RequestOriginSwitch { folder_id: FolderId },
    /// File operation for a specific folder
    FolderOperation {
        folder_id: FolderId,
        operation: FileOperation,
    },
    /// Acknowledge receipt of operation
    Ack { operation_id: u64 },
    /// Request full sync for a folder
    RequestFullSync { folder_id: FolderId },
    /// Get current user state
    GetUserState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    /// Welcome message after connection
    Welcome,
    /// Authentication successful, here's your user state
    Authenticated { user: User },
    /// New computer registered
    ComputerRegistered { computer: Computer },
    /// Sync folder created
    SyncFolderCreated { folder: SyncFolder },
    /// Joined a sync folder as backup
    JoinedSyncFolder { folder: SyncFolder },
    /// Left a sync folder
    LeftSyncFolder { folder_id: FolderId },
    /// Origin switched to a new computer
    OriginSwitched {
        folder_id: FolderId,
        new_origin: ComputerId,
    },
    /// Origin switch denied (folder not synced or requester not a backup)
    OriginSwitchDenied { folder_id: FolderId, reason: String },
    /// Forward operation to backup clients
    FolderOperation {
        folder_id: FolderId,
        operation_id: u64,
        operation: FileOperation,
    },
    /// Operation acknowledged by all backups
    OperationComplete { operation_id: u64 },
    /// Folder sync status changed
    SyncStatusChanged {
        folder_id: FolderId,
        is_synced: bool,
        pending_operations: u64,
    },
    /// Current user state
    UserState { user: User },
    /// Error message
    Error { message: String },
}
