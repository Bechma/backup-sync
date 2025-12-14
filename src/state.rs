use crate::rsync::{apply_patch, calculate_delta, create_signature};
use crate::synchronizer::Synchronizer;
use notify::event::{ModifyKind, RenameMode};
use notify::EventKind;
use notify_debouncer_full::DebouncedEvent;
use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::RwLock;

pub struct AppState {
    syncer: RwLock<Synchronizer>,
}

impl AppState {
    pub fn new(sync: Synchronizer) -> Self {
        Self {
            syncer: RwLock::new(sync),
        }
    }

    pub fn new_with_local_sync(original: &str, backup: &str) -> std::io::Result<Self> {
        let mut syncer = Synchronizer::new(original, backup)?;
        syncer.sync()?;
        Ok(Self::new(syncer))
    }

    pub fn process_debounced_event(&self, event: &DebouncedEvent) -> std::io::Result<()> {
        match event.kind {
            EventKind::Modify(ModifyKind::Data(_)) => {
                event
                    .paths
                    .par_iter()
                    .map(|x| self.process_modified_path(x))
                    .for_each(|x| x.unwrap());
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
                    .map(|x| self.process_create_path(x))
                    .for_each(|x| x.unwrap());
            }
            EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
                event
                    .paths
                    .par_iter()
                    .map(|x| self.process_delete_path(x))
                    .for_each(|x| x.unwrap());
            }
            _ => {}
        };
        Ok(())
    }

    fn process_modified_path(&self, original_path: &PathBuf) -> std::io::Result<()> {
        let mut new_file = File::open(original_path)?;
        let new_sig = create_signature(&mut new_file)?;
        let read_syncer = self.syncer.read().unwrap();
        let backup_path = read_syncer.get_backup_path(original_path).unwrap();
        let old_sig = read_syncer
            .get_backup_signature(&backup_path)
            .cloned()
            .unwrap();
        if new_sig == old_sig {
            println!("file hasn't changed: {original_path:?}");
            return Ok(());
        }
        drop(read_syncer);
        let mut write_syncer = self.syncer.write().unwrap();
        println!("file changed: {original_path:?}");
        let mut old_file = File::options().write(true).read(true).open(&backup_path)?;

        let dlt = calculate_delta(&mut new_file, &old_sig)?;

        let out = apply_patch(&mut old_file, dlt.as_slice())?;
        old_file.set_len(0)?;
        old_file.write_all(&out)?;
        old_file.sync_data()?;

        write_syncer.update_original_entry(original_path)?;
        write_syncer.update_backup_entry(&backup_path)?;
        Ok(())
    }

    fn process_create_path(&self, original_path: &PathBuf) -> std::io::Result<()> {
        let mut syncer = self.syncer.write().unwrap();
        println!("created file: {original_path:?}");
        syncer.handle_original_created(original_path.clone())
    }

    fn process_delete_path(&self, original_path: &PathBuf) -> std::io::Result<()> {
        let mut syncer = self.syncer.write().unwrap();
        println!("deleted file: {original_path:?}");
        syncer.handle_original_deleted(original_path)
    }

    fn process_rename_path(&self, from_path: &PathBuf, to_path: &PathBuf) -> std::io::Result<()> {
        let mut syncer = self.syncer.write().unwrap();
        println!("renamed file: {from_path:?} -> {to_path:?}");
        syncer.handle_original_renamed(from_path, to_path)
    }
}
