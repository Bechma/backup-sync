use crate::synchronizer::{SyncOptions, Synchronizer};
use anyhow::{Context, Result};
use notify::event::{ModifyKind, RenameMode};
use notify::EventKind;
use notify_debouncer_full::DebouncedEvent;
use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};
use std::path::PathBuf;
use std::sync::RwLock;

pub struct AppState {
    syncer: RwLock<Synchronizer>,
}

impl AppState {
    #[must_use]
    pub fn new(sync: Synchronizer) -> Self {
        Self {
            syncer: RwLock::new(sync),
        }
    }

    pub fn new_with_local_sync(
        original: PathBuf,
        backup: PathBuf,
        options: SyncOptions,
    ) -> Result<Self> {
        let mut syncer = Synchronizer::new(original.clone(), backup.clone())
            .with_context(|| {
                format!("Failed to create synchronizer for {original:?} -> {backup:?}")
            })?
            .with_options(options);
        syncer.sync().context("Failed to perform initial sync")?;
        Ok(Self::new(syncer))
    }

    pub fn process_debounced_event(&self, event: &DebouncedEvent) -> Result<()> {
        match event.kind {
            EventKind::Modify(ModifyKind::Data(_)) => {
                event
                    .paths
                    .par_iter()
                    .try_for_each(|x| self.process_modified_path(x))?;
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
                if event.paths.len() >= 2 {
                    self.process_rename_path(&event.paths[0], &event.paths[1])?;
                }
            }
            EventKind::Create(_) | EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
                event
                    .paths
                    .par_iter()
                    .try_for_each(|x| self.process_create_path(x))?;
            }
            EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
                event
                    .paths
                    .par_iter()
                    .try_for_each(|x| self.process_delete_path(x))?;
            }
            _ => {}
        }
        Ok(())
    }

    fn process_modified_path(&self, original_path: &PathBuf) -> Result<()> {
        let delta = self
            .syncer
            .read()
            .map_err(|e| anyhow::anyhow!("Failed to acquire read lock on syncer: {e}"))?
            .handle_original_modified_calculate_delta(original_path)
            .with_context(|| format!("Failed to calculate delta for: {original_path:?}"))?;
        if delta.is_empty() {
            println!("file hasn't changed: {original_path:?}");
            return Ok(());
        }
        println!("file changed: {original_path:?}");
        self.syncer
            .write()
            .map_err(|e| anyhow::anyhow!("Failed to acquire write lock on syncer: {e}"))?
            .handle_original_modified_apply_delta(original_path, &delta)
            .with_context(|| format!("Failed to apply delta for: {original_path:?}"))
    }

    fn process_create_path(&self, original_path: &PathBuf) -> Result<()> {
        println!("created file: {original_path:?}");
        self.syncer
            .write()
            .map_err(|e| anyhow::anyhow!("Failed to acquire write lock on syncer: {e}"))?
            .handle_original_created(original_path.clone())
            .with_context(|| format!("Failed to handle created file: {original_path:?}"))
    }

    fn process_delete_path(&self, original_path: &PathBuf) -> Result<()> {
        println!("deleted file: {original_path:?}");
        self.syncer
            .write()
            .map_err(|e| anyhow::anyhow!("Failed to acquire write lock on syncer: {e}"))?
            .handle_original_deleted(original_path)
            .with_context(|| format!("Failed to handle deleted file: {original_path:?}"))
    }

    fn process_rename_path(&self, from_path: &PathBuf, to_path: &PathBuf) -> Result<()> {
        println!("renamed file: {from_path:?} -> {to_path:?}");
        self.syncer
            .write()
            .map_err(|e| anyhow::anyhow!("Failed to acquire write lock on syncer: {e}"))?
            .handle_original_renamed(from_path, to_path)
            .with_context(|| format!("Failed to handle renamed file: {from_path:?} -> {to_path:?}"))
    }
}
