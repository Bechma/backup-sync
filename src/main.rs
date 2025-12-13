mod rsync;
mod synchronizer;

use crate::rsync::create_signature;
use notify::event::ModifyKind::Data;
use notify::{EventKind, RecursiveMode};
use notify_debouncer_full::{new_debouncer, DebouncedEvent};
use rayon::iter::ParallelIterator;
use rayon::prelude::IntoParallelRefIterator;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

const ORIGINAL: &str = "original";
const BACKUP: &str = "bakup";

fn main() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut debouncer = new_debouncer(std::time::Duration::from_secs(1), None, tx).unwrap();

    let mut syncer = synchronizer::Synchronizer::new(ORIGINAL, BACKUP).unwrap();
    syncer.sync().unwrap();

    let global_state = RwLock::new(syncer);

    debouncer
        .watch(Path::new(ORIGINAL), RecursiveMode::Recursive)
        .unwrap();

    while let Ok(res) = rx.recv() {
        match res {
            Ok(events) => {
                events
                    .par_iter()
                    .for_each(|x| process_debounced_event(&global_state, x).unwrap());
            }
            Err(e) => println!("watch error: {:?}", e),
        }
    }
}

fn process_debounced_event(
    global_state: &RwLock<synchronizer::Synchronizer>,
    event: &DebouncedEvent,
) -> std::io::Result<()> {
    let function = match event.kind {
        EventKind::Modify(Data(_)) => process_modified_path,
        EventKind::Create(_) => process_create_path,
        EventKind::Remove(_) => process_delete_path,
        _ => {
            return Ok(());
        }
    };
    event
        .paths
        .par_iter()
        .map(|x| function(global_state, x))
        .for_each(|x| {
            x.unwrap();
        });
    Ok(())
}

fn process_modified_path(
    global_state: &RwLock<synchronizer::Synchronizer>,
    original_path: &PathBuf,
) -> std::io::Result<()> {
    let mut new_file = File::open(original_path)?;
    let new_sig = create_signature(&mut new_file)?;
    let syncer = global_state.read().unwrap();
    let backup_path = syncer.get_backup_path(original_path).unwrap();
    let old_sig = syncer.get_backup_signature(&backup_path).cloned().unwrap();
    if new_sig == old_sig {
        println!("file hasn't changed: {original_path:?}");
        return Ok(());
    }
    drop(syncer);
    let mut syncer = global_state.write().unwrap();
    println!("file changed: {original_path:?}");
    let mut old_file = File::options().write(true).read(true).open(&backup_path)?;

    let mut dlt = Vec::<u8>::new();
    librsync::whole::delta(&mut new_file, &mut old_sig.as_slice(), &mut dlt).unwrap();

    let mut out = Vec::new();
    librsync::whole::patch(&mut old_file, &mut dlt.as_slice(), &mut out).unwrap();
    old_file.set_len(0)?;
    old_file.write_all(&out)?;
    old_file.sync_data()?;

    syncer.update_original_entry(original_path)?;
    syncer.update_backup_entry(&backup_path)?;
    Ok(())
}

fn process_create_path(
    global_state: &RwLock<synchronizer::Synchronizer>,
    original_path: &PathBuf,
) -> std::io::Result<()> {
    let mut syncer = global_state.write().unwrap();
    println!("created file: {original_path:?}");
    syncer.handle_original_created(original_path.clone())
}

fn process_delete_path(
    global_state: &RwLock<synchronizer::Synchronizer>,
    original_path: &PathBuf,
) -> std::io::Result<()> {
    let mut syncer = global_state.write().unwrap();
    println!("deleted file: {original_path:?}");
    syncer.handle_original_deleted(original_path)
}
