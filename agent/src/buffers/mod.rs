pub mod blake3_hasher;
mod delta_sync_processor;
mod file_chunk_processor;

pub use blake3_hasher::{hash_file, Hashing};
pub use delta_sync_processor::DeltaSyncProcessor;
pub use file_chunk_processor::FileChunkProcessor;

const TEMP_DIR_REF: &str = "backup_sync_temp_dir";

pub fn temp_folder_path<P: AsRef<std::path::Path>>(id: P) -> std::path::PathBuf {
    std::env::temp_dir().join(TEMP_DIR_REF).join(id)
}
