use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;

use crate::local_origin::FolderStructure;
use crate::origin::FileEntry;
use crate::rsync::{apply_patch, calculate_delta, create_signature};
use anyhow::{anyhow, Context, Result};
use fs2::FileExt;

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
        let mut new_file = File::open(original_path)
            .with_context(|| format!("Failed to open original file: {original_path:?}"))?;
        let new_sig = create_signature(&mut new_file)
            .with_context(|| format!("Failed to create signature for: {original_path:?}"))?;
        let backup_path = self
            .get_backup_path(original_path)
            .with_context(|| format!("Failed to get backup path for: {original_path:?}"))?;
        let old_sig = self.get_backup_signature(&backup_path)?;
        if new_sig == old_sig {
            return Ok(vec![]);
        }
        let dlt = calculate_delta(&mut new_file, old_sig)
            .with_context(|| format!("Failed to calculate delta for: {original_path:?}"))?;
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
        let mut old_file = File::options()
            .write(true)
            .read(true)
            .open(&backup_path)
            .with_context(|| format!("Failed to open backup file for writing: {backup_path:?}"))?;

        let out = apply_patch(&mut old_file, dlt)
            .with_context(|| format!("Failed to apply patch to: {backup_path:?}"))?;
        old_file
            .set_len(0)
            .with_context(|| format!("Failed to truncate backup file: {backup_path:?}"))?;
        old_file
            .write_all(&out)
            .with_context(|| format!("Failed to write to backup file: {backup_path:?}"))?;
        old_file
            .sync_data()
            .with_context(|| format!("Failed to sync backup file: {backup_path:?}"))?;

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

        if let Some(parent) = backup_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create parent directories for: {backup_path:?}")
            })?;
        }
        fs::copy(&original_path, &backup_path)
            .with_context(|| format!("Failed to copy {original_path:?} to {backup_path:?}"))?;

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
            if backup_path.exists() {
                fs::remove_file(&backup_path)
                    .with_context(|| format!("Failed to remove backup file: {backup_path:?}"))?;
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
    ) -> Result<()> {
        let new_backup_path = self
            .get_backup_path(to_path)
            .with_context(|| format!("Cannot determine backup path for: {to_path:?}"))?;
        let old_backup_path = self.path_mapping.remove(from_path);
        self.original.remove_entry(from_path);

        if let Some(old_backup) = old_backup_path
            && old_backup.exists()
        {
            if let Some(parent) = new_backup_path.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("Failed to create parent directories for: {new_backup_path:?}")
                })?;
            }
            fs::rename(&old_backup, &new_backup_path).with_context(|| {
                format!("Failed to rename {old_backup:?} to {new_backup_path:?}")
            })?;
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
                let file = File::open(path).with_context(|| {
                    format!("Failed to open original file for locking: {path:?}")
                })?;
                file.lock_shared()
                    .with_context(|| format!("Failed to acquire shared lock on: {path:?}"))?;
                locks.push(file);
            }
        }

        for entry in self.backup.files() {
            if !entry.is_dir() {
                let path = entry.path();
                let file = File::options()
                    .read(true)
                    .write(true)
                    .open(path)
                    .with_context(|| format!("Failed to open backup file for locking: {path:?}"))?;
                file.lock_exclusive()
                    .with_context(|| format!("Failed to acquire exclusive lock on: {path:?}"))?;
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
                    fs::create_dir_all(&backup_path).with_context(|| {
                        format!("Failed to create backup directory: {backup_path:?}")
                    })?;
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
                    fs::remove_dir_all(backup_path).with_context(|| {
                        format!("Failed to remove backup directory: {backup_path:?}")
                    })?;
                } else {
                    fs::remove_file(backup_path).with_context(|| {
                        format!("Failed to remove backup file: {backup_path:?}")
                    })?;
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
                        fs::copy(backup_path, original_path).with_context(|| {
                            format!("Failed to copy {backup_path:?} to {original_path:?}")
                        })?;
                        self.original.update_entry(original_path).with_context(|| {
                            format!("Failed to update original entry: {original_path:?}")
                        })?;
                    } else {
                        fs::copy(original_path, backup_path).with_context(|| {
                            format!("Failed to copy {original_path:?} to {backup_path:?}")
                        })?;
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
