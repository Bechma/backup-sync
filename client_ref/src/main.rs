use backup_sync_client::state;
use backup_sync_client::synchronizer::SyncOptions;
use clap::{ArgGroup, Parser};
use notify::RecursiveMode;
use notify_debouncer_full::new_debouncer;
use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    about,
    version,
    group = ArgGroup::new("sources").required(true),
    group = ArgGroup::new("backups").required(true),
)]
pub struct Cli {
    #[arg(short, long, value_name = "DIR", group = "sources")]
    source_local: Option<PathBuf>,
    #[arg(short, long, value_name = "DIR", group = "backups")]
    backup_local: Option<PathBuf>,

    #[arg(long, default_value_t = false)]
    when_missing_preserve_backup: bool,

    #[arg(long, default_value_t = false)]
    when_conflict_preserve_backup: bool,

    #[arg(long, default_value_t = false)]
    when_delete_keep_backup: bool,
}

fn main() {
    tracing_subscriber::fmt::init();

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
                Err(e) => tracing::error!("watch error: {e:?}"),
            }
        }
    }
}
