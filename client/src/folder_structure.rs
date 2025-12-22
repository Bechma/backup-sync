use crate::local_file_ops::LocalFileOps;
use crate::origin::FileEntry;
use std::collections::HashMap;
use std::collections::hash_map::{Keys, Values};
use std::fs;
use std::path::PathBuf;
use tracing::instrument;

#[derive(Debug)]
pub(crate) struct FolderStructure {
    root: PathBuf,
    entries: HashMap<PathBuf, FileEntry>,
}

impl FolderStructure {
    #[instrument(skip(root))]
    pub(crate) fn new(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let root = fs::canonicalize(root.into())?;
        let mut entries = HashMap::new();

        for entry in walkdir::WalkDir::new(&root)
            .into_iter()
            .filter_map(std::result::Result::ok)
        {
            let path = entry.path().to_path_buf();
            let metadata = fs::metadata(&path)?;

            let sig = if metadata.is_file() {
                LocalFileOps::create_signature(&path)
                    .map_err(|e| std::io::Error::other(e.to_string()))?
            } else {
                Vec::new()
            };

            let file_entry = FileEntry::new(path.clone(), sig);

            entries.insert(path, file_entry);
        }

        Ok(Self { root, entries })
    }

    pub(crate) fn root(&self) -> &PathBuf {
        &self.root
    }

    pub(crate) fn entries(&self) -> Keys<'_, PathBuf, FileEntry> {
        self.entries.keys()
    }

    pub(crate) fn files(&self) -> Values<'_, PathBuf, FileEntry> {
        self.entries.values()
    }

    pub(crate) fn get_entry(&self, path: &PathBuf) -> Option<&FileEntry> {
        self.entries.get(path)
    }

    #[instrument(skip(self))]
    pub(crate) fn update_entry(&mut self, path: &PathBuf) -> std::io::Result<()> {
        let metadata = fs::metadata(path)?;
        let sig = if metadata.is_file() {
            LocalFileOps::create_signature(path)
                .map_err(|e| std::io::Error::other(e.to_string()))?
        } else {
            Vec::new()
        };

        let file_entry = FileEntry::new(path.clone(), sig);

        self.entries.insert(path.clone(), file_entry);
        Ok(())
    }

    pub(crate) fn remove_entry(&mut self, path: &PathBuf) -> Option<FileEntry> {
        self.entries.remove(path)
    }

    #[instrument(skip(self))]
    pub(crate) fn get_relatives(&self) -> HashMap<PathBuf, PathBuf> {
        self.entries
            .keys()
            .filter_map(|p| {
                p.strip_prefix(&self.root)
                    .ok()
                    .map(|rel| (rel.to_path_buf(), p.clone()))
            })
            .collect()
    }
}
