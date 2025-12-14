mod rsync;
mod state;
mod synchronizer;

use notify::RecursiveMode;
use notify_debouncer_full::new_debouncer;
use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};
use std::path::Path;

const ORIGINAL: &str = "original";
const BACKUP: &str = "bakup";

fn main() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut debouncer = new_debouncer(std::time::Duration::from_millis(200), None, tx).unwrap();

    let global_state = state::AppState::new_with_local_sync(ORIGINAL, BACKUP).unwrap();

    debouncer
        .watch(Path::new(ORIGINAL), RecursiveMode::Recursive)
        .unwrap();

    while let Ok(res) = rx.recv() {
        match res {
            Ok(events) => {
                events
                    .par_iter()
                    .for_each(|x| global_state.process_debounced_event(x).unwrap());
            }
            Err(e) => println!("watch error: {:?}", e),
        }
    }
}
