use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;

use crate::folder_structure::FolderStructure;
use crate::local_file_ops::LocalFileOps;
use crate::origin::FileEntry;
use anyhow::{anyhow, Context, Result};

#[derive(Debug, Clone, Default)]
pub struct SyncOptions {
    when_missing_preserve_backup: bool,
    when_conflict_preserve_backup: bool,
    when_delete_keep_backup: bool,
}

impl SyncOptions {
    #[must_use]
    pub fn with_when_missing_preserve_backup(mut self, preserve: bool) -> Self {
        self.when_missing_preserve_backup = preserve;
        self
    }

    #[must_use]
    pub fn with_when_conflict_preserve_backup(mut self, when_conflict: bool) -> Self {
        self.when_conflict_preserve_backup = when_conflict;
        self
    }
    #[must_use]
    pub fn with_when_delete_keep_backup(mut self, on_delete: bool) -> Self {
        self.when_delete_keep_backup = on_delete;
        self
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
    pub fn new(original_root: PathBuf, backup_root: PathBuf) -> Result<Self> {
        let original = FolderStructure::new(&original_root).with_context(|| {
            format!("Failed to read original folder structure: {original_root:?}")
        })?;
        let backup = FolderStructure::new(&backup_root)
            .with_context(|| format!("Failed to read backup folder structure: {backup_root:?}"))?;

        let mut path_mapping = HashMap::new();

        for original_path in original.entries() {
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

    #[must_use]
    pub fn with_options(mut self, options: SyncOptions) -> Self {
        self.options = options;
        self
    }

    #[must_use]
    pub fn get_backup_path(&self, original_path: &PathBuf) -> Option<PathBuf> {
        if let Some(path) = self.path_mapping.get(original_path) {
            Some(path.clone())
        } else if let Ok(relative) = original_path.strip_prefix(self.original.root()) {
            Some(self.backup.root().join(relative))
        } else {
            None
        }
    }

    fn get_backup_signature(&self, path: &PathBuf) -> Result<&[u8]> {
        self.backup
            .get_entry(path)
            .ok_or_else(|| anyhow!("Failed to get backup signature {path:?}"))
            .map(FileEntry::signature)
    }

    pub fn handle_original_modified_calculate_delta(
        &self,
        original_path: &PathBuf,
    ) -> Result<Vec<u8>> {
        let new_sig = LocalFileOps::create_signature(original_path)?;
        let backup_path = self
            .get_backup_path(original_path)
            .with_context(|| format!("Failed to get backup path for: {original_path:?}"))?;
        let old_sig = self.get_backup_signature(&backup_path)?;
        if new_sig == old_sig {
            return Ok(vec![]);
        }
        let dlt = LocalFileOps::calculate_delta(old_sig, original_path)?;
        Ok(dlt)
    }

    pub fn handle_original_modified_apply_delta(
        &mut self,
        original_path: &PathBuf,
        dlt: &[u8],
    ) -> Result<()> {
        let backup_path = self
            .get_backup_path(original_path)
            .with_context(|| format!("Failed to get backup path for: {original_path:?}"))?;
        LocalFileOps::handle_original_modified_apply_delta(&backup_path, dlt)?;

        self.original
            .update_entry(original_path)
            .with_context(|| format!("Failed to update original entry: {original_path:?}"))?;
        self.backup
            .update_entry(&backup_path)
            .with_context(|| format!("Failed to update backup entry: {backup_path:?}"))?;
        Ok(())
    }

    pub fn handle_original_created(&mut self, original_path: PathBuf) -> Result<()> {
        let backup_path = self
            .get_backup_path(&original_path)
            .with_context(|| format!("Cannot determine backup path for: {original_path:?}"))?;

        LocalFileOps::copy_file(&original_path, &backup_path)?;

        self.original
            .update_entry(&original_path)
            .with_context(|| format!("Failed to update original entry: {original_path:?}"))?;
        self.backup
            .update_entry(&backup_path)
            .with_context(|| format!("Failed to update backup entry: {backup_path:?}"))?;
        self.path_mapping.insert(original_path, backup_path);

        Ok(())
    }

    pub fn handle_original_deleted(&mut self, original_path: &PathBuf) -> Result<()> {
        if self.options.when_delete_keep_backup {
            return Ok(());
        }
        if let Some(backup_path) = self.path_mapping.remove(original_path) {
            LocalFileOps::remove_file(&backup_path)?;
            self.backup.remove_entry(&backup_path);
        }
        self.original.remove_entry(original_path);

        Ok(())
    }

    pub fn handle_original_renamed(
        &mut self,
        from_path: &PathBuf,
        to_path: &PathBuf,
    ) -> Result<()> {
        let new_backup_path = self
            .get_backup_path(to_path)
            .with_context(|| format!("Cannot determine backup path for: {to_path:?}"))?;
        let old_backup_path = self.path_mapping.remove(from_path);
        self.original.remove_entry(from_path);

        if let Some(old_backup) = old_backup_path {
            LocalFileOps::rename_file(&old_backup, &new_backup_path)?;
            self.backup.remove_entry(&old_backup);
        }

        self.original
            .update_entry(to_path)
            .with_context(|| format!("Failed to update original entry: {to_path:?}"))?;
        self.backup
            .update_entry(&new_backup_path)
            .with_context(|| format!("Failed to update backup entry: {new_backup_path:?}"))?;
        self.path_mapping.insert(to_path.clone(), new_backup_path);

        Ok(())
    }

    pub fn sync(&mut self) -> Result<()> {
        let _locks = self
            .acquire_locks()
            .context("Failed to acquire file locks")?;

        let original_relatives = self.original.get_relatives();
        let backup_relatives = self.backup.get_relatives();

        self.sync_missing_in_backup(&original_relatives, &backup_relatives)
            .context("Failed to sync missing files in backup")?;
        self.sync_extra_in_backup(&original_relatives, &backup_relatives)
            .context("Failed to sync extra files in backup")?;
        self.sync_conflicts(&original_relatives, &backup_relatives)
            .context("Failed to sync conflicting files")?;

        Ok(())
    }

    fn acquire_locks(&self) -> Result<Vec<File>> {
        let mut locks = Vec::new();

        for entry in self.original.files() {
            if !entry.is_dir() {
                let path = entry.path();
                let file = LocalFileOps::lock_shared(path)?;
                locks.push(file);
            }
        }

        for entry in self.backup.files() {
            if !entry.is_dir() {
                let path = entry.path();
                let file = LocalFileOps::lock_exclusive(path)?;
                locks.push(file);
            }
        }

        Ok(locks)
    }

    fn sync_missing_in_backup(
        &mut self,
        original_relatives: &HashMap<PathBuf, PathBuf>,
        backup_relatives: &HashMap<PathBuf, PathBuf>,
    ) -> Result<()> {
        for (relative, original_path) in original_relatives {
            if !backup_relatives.contains_key(relative) {
                let entry = self
                    .original
                    .get_entry(original_path)
                    .with_context(|| format!("Failed to get original entry: {original_path:?}"))?;
                if entry.is_dir() {
                    let backup_path = self.backup.root().join(relative);
                    LocalFileOps::create_dir_all(&backup_path)?;
                    self.backup.update_entry(&backup_path).with_context(|| {
                        format!("Failed to update backup entry: {backup_path:?}")
                    })?;
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
    ) -> Result<()> {
        if self.options.when_missing_preserve_backup {
            return Ok(());
        }

        for (relative, backup_path) in backup_relatives {
            if !original_relatives.contains_key(relative) {
                let entry = self
                    .backup
                    .get_entry(backup_path)
                    .with_context(|| format!("Failed to get backup entry: {backup_path:?}"))?;
                if entry.is_dir() {
                    LocalFileOps::remove_dir_all(backup_path)?;
                } else {
                    LocalFileOps::remove_file(backup_path)?;
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
    ) -> Result<()> {
        for (relative, original_path) in original_relatives {
            if let Some(backup_path) = backup_relatives.get(relative) {
                let original_entry = self
                    .original
                    .get_entry(original_path)
                    .with_context(|| format!("Failed to get original entry: {original_path:?}"))?;
                let backup_entry = self
                    .backup
                    .get_entry(backup_path)
                    .with_context(|| format!("Failed to get backup entry: {backup_path:?}"))?;

                if original_entry.is_dir() || backup_entry.is_dir() {
                    continue;
                }

                if original_entry.signature() != backup_entry.signature() {
                    if self.options.when_conflict_preserve_backup {
                        LocalFileOps::copy_file(backup_path, original_path)?;
                        self.original.update_entry(original_path).with_context(|| {
                            format!("Failed to update original entry: {original_path:?}")
                        })?;
                    } else {
                        LocalFileOps::copy_file(original_path, backup_path)?;
                        self.backup.update_entry(backup_path).with_context(|| {
                            format!("Failed to update backup entry: {backup_path:?}")
                        })?;
                    }
                }
            }
        }
        Ok(())
    }
}
