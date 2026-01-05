use crate::models::{FileMetadata, RelativePath};
use blake3::Hash;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type FolderId = uuid::Uuid;
pub type OperationId = u64;

#[derive(Debug, Serialize, Deserialize)]
pub enum FileOperation {
    // Simple operations
    Delete {
        path: RelativePath,
    },
    CreateDir {
        path: RelativePath,
    },
    Rename {
        from: RelativePath,
        to: RelativePath,
    },

    // Full file transfer (small files)
    WriteFile {
        path: RelativePath,
        content: Vec<u8>,
        metadata: FileMetadata,
        hash: Hash,
    },

    // Chunked transfer (large files)
    ChunkedTransfer(ChunkedTransferOp),

    // Delta sync (rsync-style)
    DeltaSync(DeltaSyncOp),
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ChunkedTransferOp {
    Start {
        id: u64,
        total_size: u64,
        chunk_size: u64,
    },
    Chunk {
        id: u64,
        index: u64,
        data: Vec<u8>,
    },
    End {
        id: u64,
        path: RelativePath,
        hash: Hash,
        metadata: FileMetadata,
    },
    Abort {
        id: u64,
        reason: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum DeltaSyncOp {
    RequestSignature {
        path: RelativePath,
    },
    RequestDelta {
        path: RelativePath,
        signature: libsync3::Signature,
    },
    ApplyDelta {
        path: RelativePath,
        delta: libsync3::Delta,
        hash: Hash,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum FolderOperation {
    Init {
        folder_id: FolderId,
    },
    Operation {
        folder_id: FolderId,
        operation: FileOperation,
        operation_id: OperationId,
    },
    RequestSync {
        folder_id: FolderId,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum FolderResponse {
    SyncManifest(SyncManifest),
    Signature {
        path: RelativePath,
        signature: libsync3::Signature,
        hash: Hash,
    },
    Delta {
        id: u64,
        path: RelativePath,
        delta: libsync3::Delta,
        hash: Hash,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ClientMessage {
    Control(ControlMessage),
    FolderOperation(FolderOperation),
    FolderResponse(FolderResponse),
    Ack {
        operation_id: OperationId,
    },
    Error {
        operation_id: OperationId,
        message: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ServerMessage {
    Hello(Handshake),
    Control(ControlMessage),
    /// Forward operation to backup clients
    FolderOperation(FolderOperation),
    /// Response to folder operation
    FolderResponse(FolderResponse),
    /// Operation acknowledged by all backups
    Ack {
        operation_id: OperationId,
    },
    /// Error message
    Error {
        operation_id: OperationId,
        message: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Handshake {
    pub protocol_version: u64,
    pub capabilities: Vec<String>, // e.g., ["compression:zstd", "delta:rsync"]
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ControlMessage {
    Ping(u64),
    Pong(u64),
    Pause, // Backpressure
    Resume,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncManifest {
    pub folder_id: FolderId,
    pub version: u64,
    pub timestamp: i64,
    pub files: HashMap<RelativePath, FileEntry>,
    pub total_size: u64,
    pub file_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub hash: Hash,
    pub metadata: FileMetadata,
    pub chunks: libsync3::Signature,
}
