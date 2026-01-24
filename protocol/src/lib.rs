use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// Re-export all types from server-sdk for backwards compatibility
pub use backup_sync_server_sdk::{Computer, ComputerId, FolderId, SyncFolder, User, UserId};

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
    /// Start a large file transfer (Chunked upload)
    StartTransfer {
        transfer_id: u64,
        relative_path: PathBuf,
        total_size: u64,
    },
    /// A chunk of data to modify a file (rsync-style)
    FileChunk {
        transfer_id: u64,
        chunk_index: u64,
        data: Vec<u8>, // Keep this under ~64KB
    },
    /// Sent when the delta generation is done.
    /// The Backup accumulates all chunks, then applies the Delta logic using this info.
    EndTransfer {
        transfer_id: u64,
        expected_hash: String, // The Authoritative Hash calculated by Origin
    },
    /// Apply delta (Modified to include integrity check)
    ApplyDelta {
        transfer_id: u64,
        relative_path: PathBuf,
        delta: Vec<u8>,
        expected_hash: String, // Hash of the file AFTER patch is applied
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
