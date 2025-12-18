use std::path::PathBuf;

#[derive(Debug)]
pub struct FileEntry {
    path: PathBuf,
    signature: Vec<u8>,
}

impl FileEntry {
    pub(crate) fn new(path: PathBuf, signature: Vec<u8>) -> Self {
        Self { path, signature }
    }

    pub(crate) fn is_dir(&self) -> bool {
        self.signature.is_empty()
    }

    pub(crate) fn path(&self) -> &PathBuf {
        &self.path
    }

    pub(crate) fn signature(&self) -> &[u8] {
        &self.signature
    }
}
