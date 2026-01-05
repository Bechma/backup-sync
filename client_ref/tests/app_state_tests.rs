use backup_sync_client::state::AppState;
use backup_sync_client::synchronizer::SyncOptions;
use notify::EventKind;
use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};
use notify_debouncer_full::DebouncedEvent;
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::thread;
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

    let event = create_debounced_event(EventKind::Create(CreateKind::File), vec![canonical_path]);

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
    let new_file = create_file(
        original_dir.path(),
        "subdir/new_file.txt",
        "nested new content",
    );
    let canonical_path = fs::canonicalize(&new_file).unwrap();

    let event = create_debounced_event(EventKind::Create(CreateKind::File), vec![canonical_path]);

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

    let event = create_debounced_event(EventKind::Remove(RemoveKind::File), vec![canonical_path]);

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

    let event = create_debounced_event(EventKind::Remove(RemoveKind::File), vec![canonical_path]);

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
    assert_eq!(
        read_file_content(&backup_dir.path().join("file1.txt")),
        "content 1"
    );
    assert_eq!(
        read_file_content(&backup_dir.path().join("file2.txt")),
        "content 2"
    );
    assert_eq!(
        read_file_content(&backup_dir.path().join("subdir/file3.txt")),
        "content 3"
    );
}

// ==================== EDGE CASE TESTS ====================

#[test]
fn test_app_state_empty_file_sync() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "empty.txt", "");

    let _state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    let backup_file = backup_dir.path().join("empty.txt");
    assert!(backup_file.exists());
    assert_eq!(read_file_content(&backup_file), "");
}

#[test]
fn test_app_state_large_file_sync() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    let large_content: String = "x".repeat(512 * 1024); // 512KB
    create_file(original_dir.path(), "large.txt", &large_content);

    let _state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    let backup_file = backup_dir.path().join("large.txt");
    assert!(backup_file.exists());
    assert_eq!(read_file_content(&backup_file).len(), 512 * 1024);
}

#[test]
fn test_app_state_process_multiple_create_events_batch() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    let state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    // Create multiple files and process them in a single event with multiple paths
    let file1 = create_file(original_dir.path(), "batch1.txt", "content 1");
    let file2 = create_file(original_dir.path(), "batch2.txt", "content 2");
    let file3 = create_file(original_dir.path(), "batch3.txt", "content 3");

    let paths = vec![
        fs::canonicalize(&file1).unwrap(),
        fs::canonicalize(&file2).unwrap(),
        fs::canonicalize(&file3).unwrap(),
    ];

    let event = create_debounced_event(EventKind::Create(CreateKind::File), paths);

    state.process_debounced_event(&event).unwrap();

    assert!(backup_dir.path().join("batch1.txt").exists());
    assert!(backup_dir.path().join("batch2.txt").exists());
    assert!(backup_dir.path().join("batch3.txt").exists());
}

#[test]
fn test_app_state_process_multiple_delete_events_batch() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "file1.txt", "content 1");
    create_file(original_dir.path(), "file2.txt", "content 2");

    let state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    // Verify backups exist
    assert!(backup_dir.path().join("file1.txt").exists());
    assert!(backup_dir.path().join("file2.txt").exists());

    // Delete files and process batch event
    let path1 = fs::canonicalize(original_dir.path().join("file1.txt")).unwrap();
    let path2 = fs::canonicalize(original_dir.path().join("file2.txt")).unwrap();
    fs::remove_file(original_dir.path().join("file1.txt")).unwrap();
    fs::remove_file(original_dir.path().join("file2.txt")).unwrap();

    let event = create_debounced_event(EventKind::Remove(RemoveKind::File), vec![path1, path2]);

    state.process_debounced_event(&event).unwrap();

    assert!(!backup_dir.path().join("file1.txt").exists());
    assert!(!backup_dir.path().join("file2.txt").exists());
}

#[test]
fn test_app_state_process_modify_event_with_large_change() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "file.txt", "small");

    let state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    // Modify to much larger content
    let original_file = original_dir.path().join("file.txt");
    let canonical_path = fs::canonicalize(&original_file).unwrap();
    let large_content: String = "y".repeat(100 * 1024); // 100KB
    fs::write(&original_file, &large_content).unwrap();

    let event = create_debounced_event(
        EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
        vec![canonical_path],
    );

    state.process_debounced_event(&event).unwrap();

    let backup_file = backup_dir.path().join("file.txt");
    assert_eq!(read_file_content(&backup_file).len(), 100 * 1024);
}

#[test]
fn test_app_state_deeply_nested_create() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    let state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    let new_file = create_file(original_dir.path(), "a/b/c/d/e/deep.txt", "deep content");
    let canonical_path = fs::canonicalize(&new_file).unwrap();

    let event = create_debounced_event(EventKind::Create(CreateKind::File), vec![canonical_path]);

    state.process_debounced_event(&event).unwrap();

    let backup_file = backup_dir.path().join("a/b/c/d/e/deep.txt");
    assert!(backup_file.exists());
    assert_eq!(read_file_content(&backup_file), "deep content");
}

#[test]
fn test_app_state_special_characters_in_filename() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "file with spaces.txt", "content");
    create_file(original_dir.path(), "file-dashes.txt", "content");

    let _state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    assert!(backup_dir.path().join("file with spaces.txt").exists());
    assert!(backup_dir.path().join("file-dashes.txt").exists());
}

#[test]
fn test_app_state_binary_file_sync() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    let binary_content: Vec<u8> = (0..255).collect();
    let path = original_dir.path().join("binary.bin");
    fs::write(&path, &binary_content).unwrap();

    let _state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    let backup_file = backup_dir.path().join("binary.bin");
    assert!(backup_file.exists());
    assert_eq!(fs::read(&backup_file).unwrap(), binary_content);
}

#[test]
fn test_app_state_rapid_sequential_modifications() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "file.txt", "version 0");

    let state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    let original_file = original_dir.path().join("file.txt");
    let canonical_path = fs::canonicalize(&original_file).unwrap();

    // Rapid sequential modifications
    for i in 1..=10 {
        fs::write(&original_file, format!("version {i}")).unwrap();
        let event = create_debounced_event(
            EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
            vec![canonical_path.clone()],
        );
        state.process_debounced_event(&event).unwrap();
    }

    let backup_file = backup_dir.path().join("file.txt");
    assert_eq!(read_file_content(&backup_file), "version 10");
}

#[test]
fn test_app_state_create_delete_same_file() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    let state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    // Create file
    let new_file = create_file(original_dir.path(), "temp.txt", "temporary");
    let canonical_path = fs::canonicalize(&new_file).unwrap();

    let create_event = create_debounced_event(
        EventKind::Create(CreateKind::File),
        vec![canonical_path.clone()],
    );
    state.process_debounced_event(&create_event).unwrap();

    assert!(backup_dir.path().join("temp.txt").exists());

    // Delete file
    fs::remove_file(&new_file).unwrap();

    let delete_event =
        create_debounced_event(EventKind::Remove(RemoveKind::File), vec![canonical_path]);
    state.process_debounced_event(&delete_event).unwrap();

    assert!(!backup_dir.path().join("temp.txt").exists());
}

// ==================== CONCURRENCY TESTS ====================

#[test]
fn test_app_state_concurrent_create_events() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    let state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    thread::scope(|s| {
        for i in 0..5 {
            let original_path = original_dir.path();
            let state_ref = &state;
            s.spawn(move || {
                let file_path = create_file(
                    original_path,
                    &format!("concurrent_{i}.txt"),
                    &format!("content {i}"),
                );
                let canonical_path = fs::canonicalize(&file_path).unwrap();

                let event = DebouncedEvent {
                    event: notify::Event {
                        kind: EventKind::Create(CreateKind::File),
                        paths: vec![canonical_path],
                        attrs: Default::default(),
                    },
                    time: Instant::now(),
                };

                state_ref.process_debounced_event(&event).unwrap();
            });
        }
    });

    // Verify all files were created
    for i in 0..5 {
        let backup_file = backup_dir.path().join(format!("concurrent_{i}.txt"));
        assert!(backup_file.exists());
    }
}

#[test]
fn test_app_state_concurrent_modify_different_files() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    // Create initial files
    for i in 0..5 {
        create_file(
            original_dir.path(),
            &format!("file_{i}.txt"),
            &format!("initial {i}"),
        );
    }

    let state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    thread::scope(|s| {
        for i in 0..5 {
            let original_path = original_dir.path();
            let state_ref = &state;
            s.spawn(move || {
                let file_path = original_path.join(format!("file_{i}.txt"));
                let canonical_path = fs::canonicalize(&file_path).unwrap();
                fs::write(&file_path, format!("modified {i}")).unwrap();

                let event = DebouncedEvent {
                    event: notify::Event {
                        kind: EventKind::Modify(ModifyKind::Data(
                            notify::event::DataChange::Content,
                        )),
                        paths: vec![canonical_path],
                        attrs: Default::default(),
                    },
                    time: Instant::now(),
                };

                state_ref.process_debounced_event(&event).unwrap();
            });
        }
    });

    // Verify all files were modified
    for i in 0..5 {
        let backup_file = backup_dir.path().join(format!("file_{i}.txt"));
        assert_eq!(read_file_content(&backup_file), format!("modified {i}"));
    }
}

#[test]
fn test_app_state_stress_test_many_files() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    // Create many files
    for i in 0..50 {
        create_file(
            original_dir.path(),
            &format!("file_{i}.txt"),
            &format!("content {i}"),
        );
    }

    let _state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    // Verify all files synced
    for i in 0..50 {
        let backup_file = backup_dir.path().join(format!("file_{i}.txt"));
        assert!(backup_file.exists());
        assert_eq!(read_file_content(&backup_file), format!("content {i}"));
    }
}

// ==================== ERROR HANDLING TESTS ====================

#[test]
fn test_app_state_nonexistent_original_directory() {
    let backup_dir = TempDir::new().unwrap();
    let nonexistent_path = PathBuf::from("/nonexistent/path/that/does/not/exist");

    let result = AppState::new_with_local_sync(
        nonexistent_path,
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    );

    assert!(result.is_err());
}

#[test]
fn test_app_state_nonexistent_backup_directory() {
    let original_dir = TempDir::new().unwrap();
    let nonexistent_path = PathBuf::from("/nonexistent/path/that/does/not/exist");

    let result = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        nonexistent_path,
        SyncOptions::default(),
    );

    assert!(result.is_err());
}

#[test]
fn test_app_state_process_event_with_empty_paths() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    let state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    // Event with no paths should be handled gracefully
    let event = create_debounced_event(EventKind::Create(CreateKind::File), vec![]);

    let result = state.process_debounced_event(&event);
    assert!(result.is_ok());
}

#[test]
fn test_app_state_process_unknown_event_kind() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    let state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    // Access event should be ignored
    let event = create_debounced_event(EventKind::Access(notify::event::AccessKind::Read), vec![]);

    let result = state.process_debounced_event(&event);
    assert!(result.is_ok());
}

#[test]
fn test_app_state_rename_incomplete_event() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "file.txt", "content");

    let state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default(),
    )
    .unwrap();

    // Rename event with only one path (incomplete)
    let event = create_debounced_event(
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
        vec![original_dir.path().join("file.txt")],
    );

    // Should handle gracefully (not crash)
    let result = state.process_debounced_event(&event);
    assert!(result.is_ok());
}

#[test]
fn test_app_state_all_options_combined() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "original.txt", "original content");
    create_file(backup_dir.path(), "backup_only.txt", "backup content");
    create_file(original_dir.path(), "conflict.txt", "original version");
    create_file(backup_dir.path(), "conflict.txt", "backup version");

    let _state = AppState::new_with_local_sync(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
        SyncOptions::default()
            .with_when_missing_preserve_backup(true)
            .with_when_conflict_preserve_backup(true)
            .with_when_delete_keep_backup(true),
    )
    .unwrap();

    // Original file should be synced
    assert!(backup_dir.path().join("original.txt").exists());
    // Backup only file should be preserved
    assert!(backup_dir.path().join("backup_only.txt").exists());
    // Conflict should preserve backup version in original
    assert_eq!(
        read_file_content(&original_dir.path().join("conflict.txt")),
        "backup version"
    );
}
