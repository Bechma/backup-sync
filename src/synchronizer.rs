use std::collections::HashMap;
use std::fs::{self, File, Metadata};
use std::path::PathBuf;

use crate::rsync::create_signature;
use fs2::FileExt;

#[derive(Debug, Clone, Default)]
pub struct SyncOptions {
    when_missing_preserve_backup: bool,
    when_conflict_preserve_backup: bool,
    on_delete_keep_backup: bool,
}

impl SyncOptions {
    pub fn with_when_missing_preserve_backup(mut self, preserve: bool) -> Self {
        self.when_missing_preserve_backup = preserve;
        self
    }

    pub fn with_when_conflict_preserve_backup(mut self, when_conflict: bool) -> Self {
        self.when_conflict_preserve_backup = when_conflict;
        self
    }
    pub fn with_on_delete_keep_backup(mut self, on_delete: bool) -> Self {
        self.on_delete_keep_backup = on_delete;
        self
    }
}

#[derive(Debug, Clone)]
struct FileEntry {
    path: PathBuf,
    signature: Vec<u8>,
    metadata: FileMetadata,
}

#[derive(Debug, Clone)]
struct FileMetadata {
    pub size: u64,
    pub modified: Option<std::time::SystemTime>,
    pub created: Option<std::time::SystemTime>,
    pub is_dir: bool,
}

impl From<Metadata> for FileMetadata {
    fn from(meta: Metadata) -> Self {
        Self {
            size: meta.len(),
            modified: meta.modified().ok(),
            created: meta.created().ok(),
            is_dir: meta.is_dir(),
        }
    }
}

#[derive(Debug)]
struct FolderStructure {
    root: PathBuf,
    entries: HashMap<PathBuf, FileEntry>,
}

impl FolderStructure {
    pub fn new(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let root = fs::canonicalize(root.into())?;
        let mut entries = HashMap::new();

        for entry in walkdir::WalkDir::new(&root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path().to_path_buf();
            let metadata = fs::metadata(&path)?;

            let sig = if metadata.is_file() {
                let mut file = File::open(&path)?;
                create_signature(&mut file)?
            } else {
                Vec::new()
            };

            let file_entry = FileEntry {
                path: path.clone(),
                signature: sig,
                metadata: metadata.into(),
            };

            entries.insert(path, file_entry);
        }

        Ok(Self { root, entries })
    }

    pub fn get_entry(&self, path: &PathBuf) -> Option<&FileEntry> {
        self.entries.get(path)
    }

    pub fn update_entry(&mut self, path: &PathBuf) -> std::io::Result<()> {
        let metadata = fs::metadata(path)?;
        let sig = if metadata.is_file() {
            let mut file = File::open(path)?;
            create_signature(&mut file)?
        } else {
            Vec::new()
        };

        let file_entry = FileEntry {
            path: path.clone(),
            signature: sig,
            metadata: metadata.into(),
        };

        self.entries.insert(path.clone(), file_entry);
        Ok(())
    }

    pub fn remove_entry(&mut self, path: &PathBuf) -> Option<FileEntry> {
        self.entries.remove(path)
    }

    pub fn get_relatives(&self) -> HashMap<PathBuf, PathBuf> {
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

#[derive(Debug)]
pub struct Synchronizer {
    original: FolderStructure,
    backup: FolderStructure,
    path_mapping: HashMap<PathBuf, PathBuf>,
    options: SyncOptions,
}

impl Synchronizer {
    pub fn new(
        original_root: impl Into<PathBuf>,
        backup_root: impl Into<PathBuf>,
    ) -> std::io::Result<Self> {
        let original_root = fs::canonicalize(original_root.into())?;
        let backup_root = fs::canonicalize(backup_root.into())?;

        let original = FolderStructure::new(&original_root)?;
        let backup = FolderStructure::new(&backup_root)?;

        let mut path_mapping = HashMap::new();

        for original_path in original.entries.keys() {
            if let Ok(relative) = original_path.strip_prefix(&original_root) {
                let backup_path = backup_root.join(relative);
                path_mapping.insert(original_path.clone(), backup_path);
            }
        }

        Ok(Self {
            original,
            backup,
            path_mapping,
            options: SyncOptions::default(),
        })
    }

    pub fn with_options(mut self, options: SyncOptions) -> Self {
        self.options = options;
        self
    }

    pub fn get_backup_path(&self, original_path: &PathBuf) -> Option<PathBuf> {
        if let Some(path) = self.path_mapping.get(original_path) {
            Some(path.clone())
        } else if let Ok(relative) = original_path.strip_prefix(&self.original.root) {
            Some(self.backup.root.join(relative))
        } else {
            None
        }
    }

    pub fn get_original_signature(&self, path: &PathBuf) -> Option<&Vec<u8>> {
        self.original.get_entry(path).map(|e| &e.signature)
    }

    pub fn get_backup_signature(&self, path: &PathBuf) -> Option<&Vec<u8>> {
        self.backup.get_entry(path).map(|e| &e.signature)
    }

    pub fn update_original_entry(&mut self, path: &PathBuf) -> std::io::Result<()> {
        self.original.update_entry(path)
    }

    pub fn update_backup_entry(&mut self, path: &PathBuf) -> std::io::Result<()> {
        self.backup.update_entry(path)
    }

    pub fn add_path_mapping(&mut self, original_path: PathBuf, backup_path: PathBuf) {
        self.path_mapping.insert(original_path, backup_path);
    }

    pub fn remove_path_mapping(&mut self, original_path: &PathBuf) -> Option<PathBuf> {
        self.path_mapping.remove(original_path)
    }

    pub fn handle_original_created(&mut self, original_path: PathBuf) -> std::io::Result<()> {
        let backup_path = self.get_backup_path(&original_path).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Cannot determine backup path",
            )
        })?;

        if let Some(parent) = backup_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&original_path, &backup_path)?;

        self.original.update_entry(&original_path)?;
        self.backup.update_entry(&backup_path)?;
        self.path_mapping.insert(original_path, backup_path);

        Ok(())
    }

    pub fn handle_original_deleted(&mut self, original_path: &PathBuf) -> std::io::Result<()> {
        if self.options.on_delete_keep_backup {
            return Ok(());
        }
        if let Some(backup_path) = self.path_mapping.remove(original_path) {
            if backup_path.exists() {
                fs::remove_file(&backup_path)?;
            }
            self.backup.remove_entry(&backup_path);
        }
        self.original.remove_entry(original_path);

        Ok(())
    }

    pub fn handle_original_renamed(
        &mut self,
        from_path: &PathBuf,
        to_path: &PathBuf,
    ) -> std::io::Result<()> {
        let new_backup_path = self.get_backup_path(to_path).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Cannot determine backup path",
            )
        })?;
        let old_backup_path = self.path_mapping.remove(from_path);
        self.original.remove_entry(from_path);

        if let Some(old_backup) = old_backup_path
            && old_backup.exists()
        {
            if let Some(parent) = new_backup_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&old_backup, &new_backup_path)?;
            self.backup.remove_entry(&old_backup);
        }

        self.original.update_entry(to_path)?;
        self.backup.update_entry(&new_backup_path)?;
        self.path_mapping.insert(to_path.clone(), new_backup_path);

        Ok(())
    }

    pub fn sync(&mut self) -> std::io::Result<()> {
        let _locks = self.acquire_locks()?;

        let original_relatives = self.original.get_relatives();
        let backup_relatives = self.backup.get_relatives();

        self.sync_missing_in_backup(&original_relatives, &backup_relatives)?;
        self.sync_extra_in_backup(&original_relatives, &backup_relatives)?;
        self.sync_conflicts(&original_relatives, &backup_relatives)?;

        Ok(())
    }

    fn acquire_locks(&self) -> std::io::Result<Vec<File>> {
        let mut locks = Vec::new();

        for entry in self.original.entries.values() {
            if !entry.metadata.is_dir {
                let file = File::open(&entry.path)?;
                file.lock_shared()?;
                locks.push(file);
            }
        }

        for entry in self.backup.entries.values() {
            if !entry.metadata.is_dir {
                let file = File::options().read(true).write(true).open(&entry.path)?;
                file.lock_exclusive()?;
                locks.push(file);
            }
        }

        Ok(locks)
    }

    fn sync_missing_in_backup(
        &mut self,
        original_relatives: &HashMap<PathBuf, PathBuf>,
        backup_relatives: &HashMap<PathBuf, PathBuf>,
    ) -> std::io::Result<()> {
        for (relative, original_path) in original_relatives {
            if !backup_relatives.contains_key(relative) {
                let entry = self.original.get_entry(original_path).unwrap();
                if entry.metadata.is_dir {
                    let backup_path = self.backup.root.join(relative);
                    fs::create_dir_all(&backup_path)?;
                    self.backup.update_entry(&backup_path)?;
                    self.path_mapping.insert(original_path.clone(), backup_path);
                } else {
                    self.handle_original_created(original_path.clone())?;
                }
            }
        }
        Ok(())
    }

    fn sync_extra_in_backup(
        &mut self,
        original_relatives: &HashMap<PathBuf, PathBuf>,
        backup_relatives: &HashMap<PathBuf, PathBuf>,
    ) -> std::io::Result<()> {
        if self.options.when_missing_preserve_backup {
            return Ok(());
        }

        for (relative, backup_path) in backup_relatives {
            if !original_relatives.contains_key(relative) {
                let entry = self.backup.get_entry(backup_path).unwrap();
                if entry.metadata.is_dir {
                    fs::remove_dir_all(backup_path)?;
                } else {
                    fs::remove_file(backup_path)?;
                }
                self.backup.remove_entry(backup_path);
            }
        }
        Ok(())
    }

    fn sync_conflicts(
        &mut self,
        original_relatives: &HashMap<PathBuf, PathBuf>,
        backup_relatives: &HashMap<PathBuf, PathBuf>,
    ) -> std::io::Result<()> {
        for (relative, original_path) in original_relatives {
            if let Some(backup_path) = backup_relatives.get(relative) {
                let original_entry = self.original.get_entry(original_path).unwrap();
                let backup_entry = self.backup.get_entry(backup_path).unwrap();

                if original_entry.metadata.is_dir || backup_entry.metadata.is_dir {
                    continue;
                }

                if original_entry.signature != backup_entry.signature {
                    if self.options.when_conflict_preserve_backup {
                        fs::copy(backup_path, original_path)?;
                        self.original.update_entry(original_path)?;
                    } else {
                        fs::copy(original_path, backup_path)?;
                        self.backup.update_entry(backup_path)?;
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_file(dir: &std::path::Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut file = File::create(&path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        path
    }

    fn read_file_content(path: &std::path::Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn test_sync_creates_missing_file_in_backup() {
        let original_dir = TempDir::new().unwrap();
        let backup_dir = TempDir::new().unwrap();

        create_file(original_dir.path(), "file.txt", "original content");

        let mut syncer = Synchronizer::new(
            original_dir.path().to_path_buf(),
            backup_dir.path().to_path_buf(),
        )
        .unwrap();

        syncer.sync().unwrap();

        let backup_file = backup_dir.path().join("file.txt");
        assert!(backup_file.exists());
        assert_eq!(read_file_content(&backup_file), "original content");
    }

    #[test]
    fn test_sync_creates_missing_nested_file_in_backup() {
        let original_dir = TempDir::new().unwrap();
        let backup_dir = TempDir::new().unwrap();

        create_file(original_dir.path(), "subdir/nested.txt", "nested content");

        let mut syncer = Synchronizer::new(
            original_dir.path().to_path_buf(),
            backup_dir.path().to_path_buf(),
        )
        .unwrap();

        syncer.sync().unwrap();

        let backup_file = backup_dir.path().join("subdir/nested.txt");
        assert!(backup_file.exists());
        assert_eq!(read_file_content(&backup_file), "nested content");
    }

    #[test]
    fn test_sync_deletes_extra_file_in_backup() {
        let original_dir = TempDir::new().unwrap();
        let backup_dir = TempDir::new().unwrap();

        create_file(backup_dir.path(), "extra.txt", "extra content");

        let mut syncer = Synchronizer::new(
            original_dir.path().to_path_buf(),
            backup_dir.path().to_path_buf(),
        )
        .unwrap();

        syncer.sync().unwrap();

        let backup_file = backup_dir.path().join("extra.txt");
        assert!(!backup_file.exists());
    }

    #[test]
    fn test_sync_preserves_extra_file_in_backup_with_option() {
        let original_dir = TempDir::new().unwrap();
        let backup_dir = TempDir::new().unwrap();

        create_file(backup_dir.path(), "extra.txt", "extra content");

        let mut syncer = Synchronizer::new(
            original_dir.path().to_path_buf(),
            backup_dir.path().to_path_buf(),
        )
        .unwrap()
        .with_options(SyncOptions::default().with_when_missing_preserve_backup(true));
        syncer.sync().unwrap();

        let backup_file = backup_dir.path().join("extra.txt");
        assert!(backup_file.exists());
        assert_eq!(read_file_content(&backup_file), "extra content");
    }

    #[test]
    fn test_sync_overwrites_backup_on_conflict() {
        let original_dir = TempDir::new().unwrap();
        let backup_dir = TempDir::new().unwrap();

        create_file(original_dir.path(), "file.txt", "original content");
        create_file(backup_dir.path(), "file.txt", "backup content");

        let mut syncer = Synchronizer::new(
            original_dir.path().to_path_buf(),
            backup_dir.path().to_path_buf(),
        )
        .unwrap();

        syncer.sync().unwrap();

        let backup_file = backup_dir.path().join("file.txt");
        assert_eq!(read_file_content(&backup_file), "original content");
    }

    #[test]
    fn test_sync_preserves_backup_on_conflict_with_option() {
        let original_dir = TempDir::new().unwrap();
        let backup_dir = TempDir::new().unwrap();

        create_file(original_dir.path(), "file.txt", "original content");
        create_file(backup_dir.path(), "file.txt", "backup content");

        let mut syncer = Synchronizer::new(
            original_dir.path().to_path_buf(),
            backup_dir.path().to_path_buf(),
        )
        .unwrap()
        .with_options(SyncOptions::default().with_when_conflict_preserve_backup(true));
        syncer.sync().unwrap();

        let original_file = original_dir.path().join("file.txt");
        let backup_file = backup_dir.path().join("file.txt");
        assert_eq!(read_file_content(&original_file), "backup content");
        assert_eq!(read_file_content(&backup_file), "backup content");
    }

    #[test]
    fn test_sync_no_change_when_files_identical() {
        let original_dir = TempDir::new().unwrap();
        let backup_dir = TempDir::new().unwrap();

        create_file(original_dir.path(), "file.txt", "same content");
        create_file(backup_dir.path(), "file.txt", "same content");

        let mut syncer = Synchronizer::new(
            original_dir.path().to_path_buf(),
            backup_dir.path().to_path_buf(),
        )
        .unwrap();

        syncer.sync().unwrap();

        let original_file = original_dir.path().join("file.txt");
        let backup_file = backup_dir.path().join("file.txt");
        assert_eq!(read_file_content(&original_file), "same content");
        assert_eq!(read_file_content(&backup_file), "same content");
    }

    #[test]
    fn test_sync_handles_directories() {
        let original_dir = TempDir::new().unwrap();
        let backup_dir = TempDir::new().unwrap();

        fs::create_dir_all(original_dir.path().join("subdir")).unwrap();
        create_file(original_dir.path(), "subdir/file.txt", "content");

        let mut syncer = Synchronizer::new(
            original_dir.path().to_path_buf(),
            backup_dir.path().to_path_buf(),
        )
        .unwrap();

        syncer.sync().unwrap();

        assert!(backup_dir.path().join("subdir").is_dir());
        assert!(backup_dir.path().join("subdir/file.txt").exists());
    }

    #[test]
    fn test_sync_combined_operations() {
        let original_dir = TempDir::new().unwrap();
        let backup_dir = TempDir::new().unwrap();

        create_file(original_dir.path(), "only_original.txt", "original only");
        create_file(original_dir.path(), "both.txt", "original version");
        create_file(backup_dir.path(), "only_backup.txt", "backup only");
        create_file(backup_dir.path(), "both.txt", "backup version");

        let mut syncer = Synchronizer::new(
            original_dir.path().to_path_buf(),
            backup_dir.path().to_path_buf(),
        )
        .unwrap();

        syncer.sync().unwrap();

        assert!(backup_dir.path().join("only_original.txt").exists());
        assert!(!backup_dir.path().join("only_backup.txt").exists());
        assert_eq!(
            read_file_content(&backup_dir.path().join("both.txt")),
            "original version"
        );
    }

    #[test]
    fn test_handle_original_created_copies_file_to_backup() {
        let original_dir = TempDir::new().unwrap();
        let backup_dir = TempDir::new().unwrap();

        let mut syncer = Synchronizer::new(
            original_dir.path().to_path_buf(),
            backup_dir.path().to_path_buf(),
        )
        .unwrap();

        let original_file = create_file(original_dir.path(), "new_file.txt", "new content");
        let canonical_path = fs::canonicalize(&original_file).unwrap();

        syncer
            .handle_original_created(canonical_path.clone())
            .unwrap();

        let backup_file = backup_dir.path().join("new_file.txt");
        assert!(backup_file.exists());
        assert_eq!(read_file_content(&backup_file), "new content");
        assert!(syncer.path_mapping.contains_key(&canonical_path));
    }

    #[test]
    fn test_handle_original_created_creates_nested_directories() {
        let original_dir = TempDir::new().unwrap();
        let backup_dir = TempDir::new().unwrap();

        let mut syncer = Synchronizer::new(
            original_dir.path().to_path_buf(),
            backup_dir.path().to_path_buf(),
        )
        .unwrap();

        let original_file = create_file(
            original_dir.path(),
            "subdir/nested/file.txt",
            "nested content",
        );
        let canonical_path = fs::canonicalize(&original_file).unwrap();

        syncer
            .handle_original_created(canonical_path.clone())
            .unwrap();

        let backup_file = backup_dir.path().join("subdir/nested/file.txt");
        assert!(backup_file.exists());
        assert_eq!(read_file_content(&backup_file), "nested content");
    }

    #[test]
    fn test_handle_original_created_updates_entries() {
        let original_dir = TempDir::new().unwrap();
        let backup_dir = TempDir::new().unwrap();

        let mut syncer = Synchronizer::new(
            original_dir.path().to_path_buf(),
            backup_dir.path().to_path_buf(),
        )
        .unwrap();

        let original_file = create_file(original_dir.path(), "file.txt", "content");
        let canonical_path = fs::canonicalize(&original_file).unwrap();

        syncer
            .handle_original_created(canonical_path.clone())
            .unwrap();

        assert!(syncer.original.get_entry(&canonical_path).is_some());
        let backup_path = syncer.get_backup_path(&canonical_path).unwrap();
        assert!(syncer.backup.get_entry(&backup_path).is_some());
    }

    #[test]
    fn test_handle_original_deleted_removes_backup_file() {
        let original_dir = TempDir::new().unwrap();
        let backup_dir = TempDir::new().unwrap();

        let original_file = create_file(original_dir.path(), "file.txt", "content");
        create_file(backup_dir.path(), "file.txt", "content");

        let mut syncer = Synchronizer::new(
            original_dir.path().to_path_buf(),
            backup_dir.path().to_path_buf(),
        )
        .unwrap();

        let canonical_path = fs::canonicalize(&original_file).unwrap();
        fs::remove_file(&original_file).unwrap();

        syncer.handle_original_deleted(&canonical_path).unwrap();

        let backup_file = backup_dir.path().join("file.txt");
        assert!(!backup_file.exists());
        assert!(!syncer.path_mapping.contains_key(&canonical_path));
    }

    #[test]
    fn test_handle_original_deleted_removes_entries() {
        let original_dir = TempDir::new().unwrap();
        let backup_dir = TempDir::new().unwrap();

        let original_file = create_file(original_dir.path(), "file.txt", "content");
        create_file(backup_dir.path(), "file.txt", "content");

        let mut syncer = Synchronizer::new(
            original_dir.path().to_path_buf(),
            backup_dir.path().to_path_buf(),
        )
        .unwrap();

        let canonical_path = fs::canonicalize(&original_file).unwrap();
        let backup_path = syncer.get_backup_path(&canonical_path).unwrap();
        fs::remove_file(&original_file).unwrap();

        syncer.handle_original_deleted(&canonical_path).unwrap();

        assert!(syncer.original.get_entry(&canonical_path).is_none());
        assert!(syncer.backup.get_entry(&backup_path).is_none());
    }

    #[test]
    fn test_handle_original_deleted_handles_missing_backup() {
        let original_dir = TempDir::new().unwrap();
        let backup_dir = TempDir::new().unwrap();

        let original_file = create_file(original_dir.path(), "file.txt", "content");

        let mut syncer = Synchronizer::new(
            original_dir.path().to_path_buf(),
            backup_dir.path().to_path_buf(),
        )
        .unwrap();

        let canonical_path = fs::canonicalize(&original_file).unwrap();
        fs::remove_file(&original_file).unwrap();

        let result = syncer.handle_original_deleted(&canonical_path);
        assert!(result.is_ok());
    }

    #[test]
    fn test_handle_original_deleted_keeps_backup_with_option() {
        let original_dir = TempDir::new().unwrap();
        let backup_dir = TempDir::new().unwrap();

        let original_file = create_file(original_dir.path(), "file.txt", "content");
        let backup_file = create_file(backup_dir.path(), "file.txt", "content");

        let mut syncer = Synchronizer::new(
            original_dir.path().to_path_buf(),
            backup_dir.path().to_path_buf(),
        )
        .unwrap()
        .with_options(SyncOptions::default().with_on_delete_keep_backup(true));

        let canonical_path = fs::canonicalize(&original_file).unwrap();
        fs::remove_file(&original_file).unwrap();

        syncer.handle_original_deleted(&canonical_path).unwrap();

        assert!(backup_file.exists());
        assert_eq!(read_file_content(&backup_file), "content");
    }

    #[test]
    fn test_handle_original_renamed_renames_backup_file() {
        let original_dir = TempDir::new().unwrap();
        let backup_dir = TempDir::new().unwrap();

        let original_file = create_file(original_dir.path(), "old_name.txt", "content");
        create_file(backup_dir.path(), "old_name.txt", "content");

        let mut syncer = Synchronizer::new(
            original_dir.path().to_path_buf(),
            backup_dir.path().to_path_buf(),
        )
        .unwrap();

        let from_path = fs::canonicalize(&original_file).unwrap();
        let to_path = original_dir.path().join("new_name.txt");
        fs::rename(&original_file, &to_path).unwrap();
        let to_path = fs::canonicalize(&to_path).unwrap();

        syncer
            .handle_original_renamed(&from_path, &to_path)
            .unwrap();

        let old_backup = backup_dir.path().join("old_name.txt");
        let new_backup = backup_dir.path().join("new_name.txt");
        assert!(!old_backup.exists());
        assert!(new_backup.exists());
        assert_eq!(read_file_content(&new_backup), "content");
    }

    #[test]
    fn test_handle_original_renamed_updates_path_mapping() {
        let original_dir = TempDir::new().unwrap();
        let backup_dir = TempDir::new().unwrap();

        let original_file = create_file(original_dir.path(), "old_name.txt", "content");
        create_file(backup_dir.path(), "old_name.txt", "content");

        let mut syncer = Synchronizer::new(
            original_dir.path().to_path_buf(),
            backup_dir.path().to_path_buf(),
        )
        .unwrap();

        let from_path = fs::canonicalize(&original_file).unwrap();
        let to_path = original_dir.path().join("new_name.txt");
        fs::rename(&original_file, &to_path).unwrap();
        let to_path = fs::canonicalize(&to_path).unwrap();

        syncer
            .handle_original_renamed(&from_path, &to_path)
            .unwrap();

        assert!(!syncer.path_mapping.contains_key(&from_path));
        assert!(syncer.path_mapping.contains_key(&to_path));
    }

    #[test]
    fn test_handle_original_renamed_to_nested_directory() {
        let original_dir = TempDir::new().unwrap();
        let backup_dir = TempDir::new().unwrap();

        let original_file = create_file(original_dir.path(), "file.txt", "content");
        create_file(backup_dir.path(), "file.txt", "content");

        let mut syncer = Synchronizer::new(
            original_dir.path().to_path_buf(),
            backup_dir.path().to_path_buf(),
        )
        .unwrap();

        let from_path = fs::canonicalize(&original_file).unwrap();
        fs::create_dir_all(original_dir.path().join("subdir")).unwrap();
        let to_path = original_dir.path().join("subdir/renamed.txt");
        fs::rename(&original_file, &to_path).unwrap();
        let to_path = fs::canonicalize(&to_path).unwrap();

        syncer
            .handle_original_renamed(&from_path, &to_path)
            .unwrap();

        let new_backup = backup_dir.path().join("subdir/renamed.txt");
        assert!(new_backup.exists());
        assert_eq!(read_file_content(&new_backup), "content");
    }

    #[test]
    fn test_handle_original_renamed_updates_entries() {
        let original_dir = TempDir::new().unwrap();
        let backup_dir = TempDir::new().unwrap();

        let original_file = create_file(original_dir.path(), "old.txt", "content");
        create_file(backup_dir.path(), "old.txt", "content");

        let mut syncer = Synchronizer::new(
            original_dir.path().to_path_buf(),
            backup_dir.path().to_path_buf(),
        )
        .unwrap();

        let from_path = fs::canonicalize(&original_file).unwrap();
        let to_path = original_dir.path().join("new.txt");
        fs::rename(&original_file, &to_path).unwrap();
        let to_path = fs::canonicalize(&to_path).unwrap();

        syncer
            .handle_original_renamed(&from_path, &to_path)
            .unwrap();

        assert!(syncer.original.get_entry(&from_path).is_none());
        assert!(syncer.original.get_entry(&to_path).is_some());

        let new_backup = syncer.get_backup_path(&to_path).unwrap();
        assert!(syncer.backup.get_entry(&new_backup).is_some());
    }
}
