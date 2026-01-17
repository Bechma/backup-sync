This is an agent that have the following actors:

- WebsocketHandler: Handles the websocket connection, dispatches messages and listen from new ones:
    - When it receives a message from the server it needs to send it to the FolderHandler
    - When it receives a message from the FolderHandler it needs to send it to the server
    - The interaction needs to be thread-safe and concurrent.
- FolderHandler: Handles messages from the websocket and dispatches them to the appropriate actor:
    - Delta handler: Ephemeral handler that will be created for the transfer_id assigned from the server. It's created
      from FolderHandler to handle the transactions from the Websocket. It will have a channel to receive the messages
      directly from the WebsocketHandler. When finished, it will inform about it's ending to the FolderHandler.
    - Chunk handler: Same as delta handler, but for handling file chunks operations for big files transfers.
    - File handler: Same as delta handler but for handling file operations.
    - Delta processor: Ephemeral handler that will create the deltas from the original file and the modified file.
    - Chunk processor: Ephemeral handler that will read a file and send it to the server in chunks(or altogether if it's
      less than a specified amount like 64KB by default).
    - The interaction needs to be thread-safe and concurrent.
    - It needs to protect himself from operation duplication:
        - If an operation from the server made a modification in file or folder 1, it needs to drop the operation from
          the NotifyHandler that will receive informing about the change in file or folder 1.
- NotifyHandler: Listen for file system changes events and dispatches them to FolderHandler.
    - The interaction needs to be thread-safe and concurrent.

The interaction between the actors needs to be thread-safe and concurrent.

The library for deltas is a custom one that it's already in `Cargo.toml`

If you need an async runtime will be tokio with tokio-tungstenite for the websocket connection.

The deduplication strategy will be time-based, so the changed file will be kept in memory for 1 second. If there's an
event from NotifyHandler inside of this window, it will be dropped.

The messages from the server can arrive unordered. So you can't expect a start-chunk1-chunk2-end for instance. The first
message will always be start and after that, the end might arrive before than some chunks. This means that when a
message is received and processed correctly, the handler needs to send an acknowledgement to the Websocket handler to
inform
that it's ready to receive the next message.

The ephemeral handler lifecycle run to completion for a single transfer. This single transfer includes the start of the
transfer, where this ephemeral handler will be created, the chunks of the transfer and the end of the transfer. Once the
handler finishes with all filesystem operations, it will inform the FolderHandler about it's ending so FolderHandler can
drop him after doing the processing of the ending.

The transfers always start from the agent side. The server would be just a coordinator.

The resolution of conflicts will be the following:

- If the file is modified while a transfer is on the way, we need to start a new transfer but we'll append "_
  conflict-{timestamp}" to the file name(keep the extension).
- We need to keep track of the timestamp when the file changed. So the transfer will happen but the oldest one will
  remain as original, and the new one will be the one with the timestamp.
- If the file is deleted while a transfer is on the way, we need to abort the transfer.
- The timestamp is the mtime of the file.
- Do not follow symlinks, hardlink or special files. Ignore them completely.
- Always preserve the metadata and permissions of the file.

Backpressure for the FolderHandler needs to handle all messages in real time, it's a coordinator so the heavy work is
handled to the other actors.

When choosing between delta and chunk transfer, the difference is that delta will be for files that already exists and
chunk is for new files.

When initializing a new "folder", there's a thing called manifest(similar to steam ones). That will contain the
signatures of all files in the folder and their corresponding subfolders. This way, if a file is modified, we can
calculate the delta from the previous signature straight away. If a file is modified while a transfer is on the way, we
need to abort the transfer and start a new one.

The message protocol used will be postcard.

The `notify` crate is recommended to use with recursive watching enabled.

The agent can have multiple folders to watch as it will only be one agent per pc. But only one websocket connection

If the websocket connection is closed, all transfers to the websocket needs to stop until the connection is restored, so
all transfers can resume. The notify events/actions will queue in the FolderHandler until the connection is restored.

We can have ignored patterns specified per folder.

The timeout for transfers is 10 seconds. If a transfer didn't receive any message in this period, it will be aborted.

```rust
pub enum FileType {
    File,
    Directory,
    Other,
}

pub struct Permissions {
    /// Unix mode bits (e.g., 0o755). On Windows, synthesized from attributes.
    mode: u32,
    /// Read-only flag (cross-platform)
    readonly: bool,
    /// Hidden file (Windows native, Unix: starts with dot)
    hidden: bool,
}

pub struct FileMetadata {
    file_type: FileType,
    size: u64,
    permissions: Permissions,
    mtime: time::OffsetDateTime,
}

pub struct RelativePath(String);

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
        hash: blake3::Hash,
        metadata: FileMetadata,
    },
    Abort {
        id: u64,
        reason: String,
    },
}

pub enum DeltaSyncOp {
    TransferStart {
        id: u64,
        path: RelativePath,
        total_chunks: usize,
    },
    TransferChunk {
        id: u64,
        chunk_index: u64,
        chunk: Vec<libsync3::DeltaCommand>,
    },
    TransferEnd {
        id: u64,
        hash: Hash,
    },
    Abort {
        id: u64,
        reason: String,
    },
}

pub enum FileOp {
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

    // Signature
    RequestSignature {
        path: RelativePath,
    },
    ResponseSignature {
        path: RelativePath,
        signature: libsync3::Signatures,
    },
}
```