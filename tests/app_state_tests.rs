use backup_sync::state::AppState;
use backup_sync::synchronizer::SyncOptions;
use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};
use notify::EventKind;
use notify_debouncer_full::DebouncedEvent;
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;
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

fn create_debounced_event(kind: EventKind, paths: Vec<PathBuf>) -> DebouncedEvent {
    DebouncedEvent {
        event: notify::Event {
            kind,
            paths: paths.clone(),
            attrs: Default::default(),
        },
        time: Instant::now(),
    }
}

#[test]
fn test_app_state_new_with_local_sync_creates_backup() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "file.txt", "content");

    let _state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    let backup_file = backup_dir.path().join("file.txt");
    assert!(backup_file.exists());
    assert_eq!(read_file_content(&backup_file), "content");
}

#[test]
fn test_app_state_new_with_local_sync_nested_files() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "subdir/nested.txt", "nested content");

    let _state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    let backup_file = backup_dir.path().join("subdir/nested.txt");
    assert!(backup_file.exists());
    assert_eq!(read_file_content(&backup_file), "nested content");
}

#[test]
fn test_app_state_process_create_event() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    let state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    // Create a new file after initial sync
    let new_file = create_file(original_dir.path(), "new_file.txt", "new content");
    let canonical_path = fs::canonicalize(&new_file).unwrap();

    let event = create_debounced_event(
        EventKind::Create(CreateKind::File),
        vec![canonical_path],
    );

    state.process_debounced_event(&event).unwrap();

    let backup_file = backup_dir.path().join("new_file.txt");
    assert!(backup_file.exists());
    assert_eq!(read_file_content(&backup_file), "new content");
}

#[test]
fn test_app_state_process_create_event_nested() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    let state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    // Create a new nested file after initial sync
    let new_file = create_file(original_dir.path(), "subdir/new_file.txt", "nested new content");
    let canonical_path = fs::canonicalize(&new_file).unwrap();

    let event = create_debounced_event(
        EventKind::Create(CreateKind::File),
        vec![canonical_path],
    );

    state.process_debounced_event(&event).unwrap();

    let backup_file = backup_dir.path().join("subdir/new_file.txt");
    assert!(backup_file.exists());
    assert_eq!(read_file_content(&backup_file), "nested new content");
}

#[test]
fn test_app_state_process_delete_event() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "file.txt", "content");

    let state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    // Verify backup exists
    let backup_file = backup_dir.path().join("file.txt");
    assert!(backup_file.exists());

    // Delete the original file
    let original_file = original_dir.path().join("file.txt");
    let canonical_path = fs::canonicalize(&original_file).unwrap();
    fs::remove_file(&original_file).unwrap();

    let event = create_debounced_event(
        EventKind::Remove(RemoveKind::File),
        vec![canonical_path],
    );

    state.process_debounced_event(&event).unwrap();

    assert!(!backup_file.exists());
}

#[test]
fn test_app_state_process_delete_event_keeps_backup_with_option() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "file.txt", "content");

    let state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default().with_when_delete_keep_backup(true),
    )
    .unwrap();

    // Verify backup exists
    let backup_file = backup_dir.path().join("file.txt");
    assert!(backup_file.exists());

    // Delete the original file
    let original_file = original_dir.path().join("file.txt");
    let canonical_path = fs::canonicalize(&original_file).unwrap();
    fs::remove_file(&original_file).unwrap();

    let event = create_debounced_event(
        EventKind::Remove(RemoveKind::File),
        vec![canonical_path],
    );

    state.process_debounced_event(&event).unwrap();

    // Backup should still exist due to option
    assert!(backup_file.exists());
    assert_eq!(read_file_content(&backup_file), "content");
}

#[test]
fn test_app_state_process_rename_event() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "old_name.txt", "content");

    let state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    // Verify backup exists
    let old_backup = backup_dir.path().join("old_name.txt");
    assert!(old_backup.exists());

    // Rename the original file
    let old_path = original_dir.path().join("old_name.txt");
    let from_path = fs::canonicalize(&old_path).unwrap();
    let to_path = original_dir.path().join("new_name.txt");
    fs::rename(&old_path, &to_path).unwrap();
    let to_path = fs::canonicalize(&to_path).unwrap();

    let event = create_debounced_event(
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
        vec![from_path, to_path],
    );

    state.process_debounced_event(&event).unwrap();

    let new_backup = backup_dir.path().join("new_name.txt");
    assert!(!old_backup.exists());
    assert!(new_backup.exists());
    assert_eq!(read_file_content(&new_backup), "content");
}

#[test]
fn test_app_state_process_modify_event() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "file.txt", "original content");

    let state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    // Verify backup exists with original content
    let backup_file = backup_dir.path().join("file.txt");
    assert_eq!(read_file_content(&backup_file), "original content");

    // Modify the original file
    let original_file = original_dir.path().join("file.txt");
    let canonical_path = fs::canonicalize(&original_file).unwrap();
    fs::write(&original_file, "modified content").unwrap();

    let event = create_debounced_event(
        EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
        vec![canonical_path],
    );

    state.process_debounced_event(&event).unwrap();

    assert_eq!(read_file_content(&backup_file), "modified content");
}

#[test]
fn test_app_state_process_modify_event_no_change() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "file.txt", "same content");

    let state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    // Verify backup exists
    let backup_file = backup_dir.path().join("file.txt");
    assert_eq!(read_file_content(&backup_file), "same content");

    // Trigger modify event without actually changing content
    let original_file = original_dir.path().join("file.txt");
    let canonical_path = fs::canonicalize(&original_file).unwrap();

    let event = create_debounced_event(
        EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
        vec![canonical_path],
    );

    // Should not error even though content is the same
    state.process_debounced_event(&event).unwrap();

    assert_eq!(read_file_content(&backup_file), "same content");
}

#[test]
fn test_app_state_process_rename_to_event() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    let state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    // Create a new file (simulating RenameMode::To which is treated like Create)
    let new_file = create_file(original_dir.path(), "moved_in.txt", "moved content");
    let canonical_path = fs::canonicalize(&new_file).unwrap();

    let event = create_debounced_event(
        EventKind::Modify(ModifyKind::Name(RenameMode::To)),
        vec![canonical_path],
    );

    state.process_debounced_event(&event).unwrap();

    let backup_file = backup_dir.path().join("moved_in.txt");
    assert!(backup_file.exists());
    assert_eq!(read_file_content(&backup_file), "moved content");
}

#[test]
fn test_app_state_process_rename_from_event() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "file.txt", "content");

    let state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    // Verify backup exists
    let backup_file = backup_dir.path().join("file.txt");
    assert!(backup_file.exists());

    // Remove the original file (simulating RenameMode::From which is treated like Remove)
    let original_file = original_dir.path().join("file.txt");
    let canonical_path = fs::canonicalize(&original_file).unwrap();
    fs::remove_file(&original_file).unwrap();

    let event = create_debounced_event(
        EventKind::Modify(ModifyKind::Name(RenameMode::From)),
        vec![canonical_path],
    );

    state.process_debounced_event(&event).unwrap();

    assert!(!backup_file.exists());
}

#[test]
fn test_app_state_with_sync_options_preserve_backup() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    // Create extra file in backup that doesn't exist in original
    create_file(backup_dir.path(), "extra.txt", "extra content");

    let _state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default().with_when_missing_preserve_backup(true),
    )
    .unwrap();

    // Extra file should be preserved
    let extra_file = backup_dir.path().join("extra.txt");
    assert!(extra_file.exists());
    assert_eq!(read_file_content(&extra_file), "extra content");
}

#[test]
fn test_app_state_with_sync_options_conflict_preserve_backup() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "file.txt", "original content");
    create_file(backup_dir.path(), "file.txt", "backup content");

    let _state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default().with_when_conflict_preserve_backup(true),
    )
    .unwrap();

    // Original should be overwritten with backup content
    let original_file = original_dir.path().join("file.txt");
    let backup_file = backup_dir.path().join("file.txt");
    assert_eq!(read_file_content(&original_file), "backup content");
    assert_eq!(read_file_content(&backup_file), "backup content");
}

#[test]
fn test_app_state_multiple_files_sync() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "file1.txt", "content 1");
    create_file(original_dir.path(), "file2.txt", "content 2");
    create_file(original_dir.path(), "subdir/file3.txt", "content 3");

    let _state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    assert!(backup_dir.path().join("file1.txt").exists());
    assert!(backup_dir.path().join("file2.txt").exists());
    assert!(backup_dir.path().join("subdir/file3.txt").exists());
    assert_eq!(read_file_content(&backup_dir.path().join("file1.txt")), "content 1");
    assert_eq!(read_file_content(&backup_dir.path().join("file2.txt")), "content 2");
    assert_eq!(read_file_content(&backup_dir.path().join("subdir/file3.txt")), "content 3");
}
