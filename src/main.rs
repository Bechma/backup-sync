use backup_sync::state;
use backup_sync::synchronizer::SyncOptions;
use clap::Parser;
use notify::RecursiveMode;
use notify_debouncer_full::new_debouncer;
use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};
use std::path::PathBuf;

#[derive(Parser)]
#[command(about, version)]
pub struct Cli {
    #[arg(short, long, value_name = "DIR")]
    source_local: Option<PathBuf>,
    #[arg(short, long, value_name = "DIR")]
    backup_local: Option<PathBuf>,

    #[arg(long, default_value_t = false)]
    when_missing_preserve_backup: bool,

    #[arg(long, default_value_t = false)]
    when_conflict_preserve_backup: bool,

    #[arg(long, default_value_t = false)]
    when_delete_keep_backup: bool,
}

fn main() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut debouncer = new_debouncer(std::time::Duration::from_millis(200), None, tx).unwrap();

    let cli = Cli::parse();
    let options = SyncOptions::default()
        .with_when_delete_keep_backup(cli.when_delete_keep_backup)
        .with_when_conflict_preserve_backup(cli.when_conflict_preserve_backup)
        .with_when_missing_preserve_backup(cli.when_missing_preserve_backup);

    if let Some(source) = cli.source_local
        && let Some(backup) = cli.backup_local
    {
        debouncer.watch(&source, RecursiveMode::Recursive).unwrap();
        let global_state = state::AppState::new_with_local_sync(source, backup, options).unwrap();

        while let Ok(res) = rx.recv() {
            match res {
                Ok(events) => {
                    events
                        .par_iter()
                        .for_each(|x| global_state.process_debounced_event(x).unwrap());
                }
                Err(e) => println!("watch error: {e:?}"),
            }
        }
    }
}
