# Definition

An agent is a piece of software that will be on charge of applying the changes received from the server or to inform the server about the changes in the local machine.

It will not inform about all the changes though, but only the specific folders specified to the server.
An example of it would be:

- Agent 1 configure to synchronize a folder named backup1 in the path /home/user/Documents
- Agent 1 synchronize with the server that folder.
- Agent 2 wants to have that folder synchronized in its local machine in the path C:\Users\windowsuser\synced_documents
- Agent 2 join the folder synchronization backup1 in the path C:\Users\windowsuser\synced_documents
- Server will ask:
    - Agent 1 for the files that Agent 2 doesn't have or they have a newer version(mtime is newer).
    - Agent 2 for the files that Agent 1 doesn't have or they have a newer version(mtime is newer).

# Components

The agent have the following actors:

- WebsocketHandler: Handles the websocket connection, dispatches messages and listen from new ones:
    - When it receives a message from the server it needs to send it to the FolderHandler
    - When it receives a message from the FolderHandler it needs to send it to the server
    - The interaction needs to be thread-safe and concurrent.
- FolderHandler: Handles messages from the websocket and dispatches them to the appropriate actor:
    - Delta handler: Ephemeral handler that will be created for the transfer_id assigned from the originated agent.
    It's created from FolderHandler to handle the transactions from the Websocket.
    It will have a channel to receive the messages directly from the WebsocketHandler.
    When finished, it will inform about it's ending to the FolderHandler.
    - Chunk handler: Same as delta handler, but for handling file chunks operations for big files transfers.
    - File handler: Simple file operations and/or signature calculation.
    - Delta processor: Ephemeral handler that will create the deltas from the original file and the modified file.
    - Chunk processor: Ephemeral handler that will read a file and send it to the server in chunks(or altogether if it's less than a specified amount like 64KB by default).
    - The interaction needs to be thread-safe and concurrent.
    - It needs to protect himself from operation duplication:
        - If an operation from the server made a modification in file or folder 1, it needs to drop the operation from
          the NotifyHandler that will receive informing about the change in file or folder 1.
- NotifyHandler: Listen for file system changes events and dispatches them to FolderHandler.
    - The interaction needs to be thread-safe and concurrent.

The interaction between the actors needs to be thread-safe and concurrent.

# Constraints

## Delta transfer calculation

The library for deltas is not an existing one, but one develop for this specific use-case called libsync3.

It will be used to calculate the deltas from the original file and the modified file.


```rust
pub enum FileType {
    File,
    Directory,
    Other,
}

pub struct Permissions {
    /// Unix mode bits (e.g., 0o755). On Windows, synthesized from attributes.
    mode: u32,
    readonly: bool,
    hidden: bool,
}

pub struct FileMetadata {
    file_type: FileType,
    size: u64,
    permissions: Permissions,
    mtime: time::OffsetDateTime,
}

pub struct RelativePath(String);

pub enum FileTransferOp {
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

pub enum DeltaTransferOp {
    Start {
        id: u64,
        path: RelativePath,
        total_chunks: usize,
    },
    Chunk {
        id: u64,
        chunk_index: u64,
        chunk: Vec<libsync3::DeltaCommand>,
    },
    End {
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
}
```

## Deduplication strategy

The deduplication strategy will be time-based debouncing, so the changed file will be kept in memory for 1 second.
If there's an event from NotifyHandler inside of this window, it will be dropped.

## Message ordering

The messages from the server can arrive unordered.
So you can't expect a start-chunk1-chunk2-end for instance.
The first message will always be start and after that, the end might arrive before than some chunks.
This means that when a message is received and processed correctly, the handler needs to send an acknowledgement to the Websocket handler to inform that it's ready to receive the next message.

The acknowledgement will be per message. So the originator of the chunks will not release the chunk until it receives the acknowledgement, so a retry is possible without reading the file again.

## Ephemeral handler lifecycle

The ephemeral handler lifecycle run to completion for a single transfer.
This single transfer includes the start of the transfer, where this ephemeral handler will be created, the chunks of the transfer and the end of the transfer.
Once the handler finishes with all filesystem operations, it will inform the FolderHandler about it's ending so FolderHandler can drop him after doing the processing of the ending.

## Transfer direction

The transfers always start from the agent side. The server would be just a coordinator and validator.

When there's an interruption in the transfer, the flow will resume.

## Conflict resolution

The resolution of conflicts will be the following:

- If the file is modified while a transfer is on the way, we need to start a new transfer but we'll append "_
  conflict-{timestamp}" to the file name(keep the extension). timestamp in the format of 2026-01-18T23:00:00Z
- We need to keep track of the timestamp when the file changed. So the transfer will happen but the oldest one will
  remain as original, and the new one will be the one with the timestamp.
- If the file is deleted while a transfer is on the way, we need to abort the transfer.
- The timestamp is the mtime of the file transformed into a `time::OffsetDateTime` so it will be on the same format as the server.
- Do not follow symlinks, hardlink or special files. Ignore them completely.
- Always preserve the metadata and permissions of the file.
- If there's a mismatch on the hash when received the file, the agent will retry the transfer.

Backpressure for the FolderHandler needs to handle all messages in real time, it's a coordinator so the heavy work is handled to the other actors.

## Transfer selection

When choosing between delta and chunk transfer, the difference is that delta will be for files that already exists and chunk is for new files.

## Manifest

When initializing a new "folder", there's a thing called manifest(similar to steam ones).
This will contain the signatures of all files in the folder and their corresponding subfolders.
This way, if a file is modified, we can calculate the delta from the previous signature straight away.
If a file is modified while a transfer is on the way, we need to abort the transfer and start a new one.

```rust
pub struct FileEntry {
    hash: blake3::Hash,
    metadata: FileMetadata,
    signature: libsync3::Signatures,
}

pub struct Manifest {
    folder_id: String,
    timestamp: i64,
    files: HashMap<RelativePath, FileEntry>,
}

pub struct ManifestLight {
    folder_id: String,
    timestamp: i64,
    files: HashMap<RelativePath, FileMetadata>,
}
```

The manifest will be stored in the database as the ground of truth for the folder.
When a new client connect to the sync folder the flow will be:
- It will request the manifest from the server.
- The server will send a light current manifest to the client.
- The client will request the new files to the chain and the files that were modified.
  If an existing file was modified with a latter timestamp in the client, the client will send a delta request to the server.
  If nothing is sent back to the server, the server must suppose that this client is on-sync.
- The synchronization flow will start just after the client received the light manifest.

## File system events

The agent can have multiple folders to watch as it will only be one agent per pc. But only one websocket connection.

If the websocket connection is closed, all transfers to the websocket needs to stop until the connection is restored, so all transfers can resume.
The notify events/actions will queue in the FolderHandler until the connection is restored.

We can have gitignore-style ignored patterns specified per folder.

The timeout for transfers is 10 seconds. If a transfer didn't receive any message in this period, it will be aborted.
