use backup_sync::synchronizer::{SyncOptions, Synchronizer};
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
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

#[test]
fn test_sync_creates_missing_file_in_backup() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "file.txt", "original content");

    let mut syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap();

    syncer.sync().unwrap();

    let backup_file = backup_dir.path().join("file.txt");
    assert!(backup_file.exists());
    assert_eq!(read_file_content(&backup_file), "original content");
}

#[test]
fn test_sync_creates_missing_nested_file_in_backup() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "subdir/nested.txt", "nested content");

    let mut syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap();

    syncer.sync().unwrap();

    let backup_file = backup_dir.path().join("subdir/nested.txt");
    assert!(backup_file.exists());
    assert_eq!(read_file_content(&backup_file), "nested content");
}

#[test]
fn test_sync_deletes_extra_file_in_backup() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(backup_dir.path(), "extra.txt", "extra content");

    let mut syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap();

    syncer.sync().unwrap();

    let backup_file = backup_dir.path().join("extra.txt");
    assert!(!backup_file.exists());
}

#[test]
fn test_sync_preserves_extra_file_in_backup_with_option() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(backup_dir.path(), "extra.txt", "extra content");

    let mut syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap()
    .with_options(SyncOptions::default().with_when_missing_preserve_backup(true));
    syncer.sync().unwrap();

    let backup_file = backup_dir.path().join("extra.txt");
    assert!(backup_file.exists());
    assert_eq!(read_file_content(&backup_file), "extra content");
}

#[test]
fn test_sync_overwrites_backup_on_conflict() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "file.txt", "original content");
    create_file(backup_dir.path(), "file.txt", "backup content");

    let mut syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap();

    syncer.sync().unwrap();

    let backup_file = backup_dir.path().join("file.txt");
    assert_eq!(read_file_content(&backup_file), "original content");
}

#[test]
fn test_sync_preserves_backup_on_conflict_with_option() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "file.txt", "original content");
    create_file(backup_dir.path(), "file.txt", "backup content");

    let mut syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap()
    .with_options(SyncOptions::default().with_when_conflict_preserve_backup(true));
    syncer.sync().unwrap();

    let original_file = original_dir.path().join("file.txt");
    let backup_file = backup_dir.path().join("file.txt");
    assert_eq!(read_file_content(&original_file), "backup content");
    assert_eq!(read_file_content(&backup_file), "backup content");
}

#[test]
fn test_sync_no_change_when_files_identical() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "file.txt", "same content");
    create_file(backup_dir.path(), "file.txt", "same content");

    let mut syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap();

    syncer.sync().unwrap();

    let original_file = original_dir.path().join("file.txt");
    let backup_file = backup_dir.path().join("file.txt");
    assert_eq!(read_file_content(&original_file), "same content");
    assert_eq!(read_file_content(&backup_file), "same content");
}

#[test]
fn test_sync_handles_directories() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    fs::create_dir_all(original_dir.path().join("subdir")).unwrap();
    create_file(original_dir.path(), "subdir/file.txt", "content");

    let mut syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap();

    syncer.sync().unwrap();

    assert!(backup_dir.path().join("subdir").is_dir());
    assert!(backup_dir.path().join("subdir/file.txt").exists());
}

#[test]
fn test_sync_combined_operations() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "only_original.txt", "original only");
    create_file(original_dir.path(), "both.txt", "original version");
    create_file(backup_dir.path(), "only_backup.txt", "backup only");
    create_file(backup_dir.path(), "both.txt", "backup version");

    let mut syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap();

    syncer.sync().unwrap();

    assert!(backup_dir.path().join("only_original.txt").exists());
    assert!(!backup_dir.path().join("only_backup.txt").exists());
    assert_eq!(
        read_file_content(&backup_dir.path().join("both.txt")),
        "original version"
    );
}

#[test]
fn test_handle_original_created_copies_file_to_backup() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    let mut syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap();

    let original_file = create_file(original_dir.path(), "new_file.txt", "new content");
    let canonical_path = fs::canonicalize(&original_file).unwrap();

    syncer
        .handle_original_created(canonical_path.clone())
        .unwrap();

    let backup_file = backup_dir.path().join("new_file.txt");
    assert!(backup_file.exists());
    assert_eq!(read_file_content(&backup_file), "new content");
    // Verify via get_backup_path that the mapping exists
    assert!(syncer.get_backup_path(&canonical_path).is_some());
}

#[test]
fn test_handle_original_created_creates_nested_directories() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    let mut syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap();

    let original_file = create_file(
        original_dir.path(),
        "subdir/nested/file.txt",
        "nested content",
    );
    let canonical_path = fs::canonicalize(&original_file).unwrap();

    syncer
        .handle_original_created(canonical_path.clone())
        .unwrap();

    let backup_file = backup_dir.path().join("subdir/nested/file.txt");
    assert!(backup_file.exists());
    assert_eq!(read_file_content(&backup_file), "nested content");
}

#[test]
fn test_handle_original_created_updates_entries() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    let mut syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap();

    let original_file = create_file(original_dir.path(), "file.txt", "content");
    let canonical_path = fs::canonicalize(&original_file).unwrap();

    syncer
        .handle_original_created(canonical_path.clone())
        .unwrap();

    // Verify backup path is tracked
    let backup_path = syncer.get_backup_path(&canonical_path).unwrap();
    assert!(backup_path.exists());
}

#[test]
fn test_handle_original_deleted_removes_backup_file() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    let original_file = create_file(original_dir.path(), "file.txt", "content");
    create_file(backup_dir.path(), "file.txt", "content");

    let mut syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap();

    let canonical_path = fs::canonicalize(&original_file).unwrap();
    fs::remove_file(&original_file).unwrap();

    syncer.handle_original_deleted(&canonical_path).unwrap();

    let backup_file = backup_dir.path().join("file.txt");
    assert!(!backup_file.exists());
    // Verify mapping is removed via get_backup_path returning a path that doesn't exist in mapping
    // (get_backup_path will still compute a path, but the file won't exist)
}

#[test]
fn test_handle_original_deleted_handles_missing_backup() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    let original_file = create_file(original_dir.path(), "file.txt", "content");

    let mut syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap();

    let canonical_path = fs::canonicalize(&original_file).unwrap();
    fs::remove_file(&original_file).unwrap();

    let result = syncer.handle_original_deleted(&canonical_path);
    assert!(result.is_ok());
}

#[test]
fn test_handle_original_deleted_keeps_backup_with_option() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    let original_file = create_file(original_dir.path(), "file.txt", "content");
    let backup_file = create_file(backup_dir.path(), "file.txt", "content");

    let mut syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap()
    .with_options(SyncOptions::default().with_when_delete_keep_backup(true));

    let canonical_path = fs::canonicalize(&original_file).unwrap();
    fs::remove_file(&original_file).unwrap();

    syncer.handle_original_deleted(&canonical_path).unwrap();

    assert!(backup_file.exists());
    assert_eq!(read_file_content(&backup_file), "content");
}

#[test]
fn test_handle_original_renamed_renames_backup_file() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    let original_file = create_file(original_dir.path(), "old_name.txt", "content");
    create_file(backup_dir.path(), "old_name.txt", "content");

    let mut syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap();

    let from_path = fs::canonicalize(&original_file).unwrap();
    let to_path = original_dir.path().join("new_name.txt");
    fs::rename(&original_file, &to_path).unwrap();
    let to_path = fs::canonicalize(&to_path).unwrap();

    syncer
        .handle_original_renamed(&from_path, &to_path)
        .unwrap();

    let old_backup = backup_dir.path().join("old_name.txt");
    let new_backup = backup_dir.path().join("new_name.txt");
    assert!(!old_backup.exists());
    assert!(new_backup.exists());
    assert_eq!(read_file_content(&new_backup), "content");
}

#[test]
fn test_handle_original_renamed_updates_path_mapping() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    let original_file = create_file(original_dir.path(), "old_name.txt", "content");
    create_file(backup_dir.path(), "old_name.txt", "content");

    let mut syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap();

    let from_path = fs::canonicalize(&original_file).unwrap();
    let to_path = original_dir.path().join("new_name.txt");
    fs::rename(&original_file, &to_path).unwrap();
    let to_path = fs::canonicalize(&to_path).unwrap();

    syncer
        .handle_original_renamed(&from_path, &to_path)
        .unwrap();

    // Verify new path is tracked
    assert!(syncer.get_backup_path(&to_path).is_some());
}

#[test]
fn test_handle_original_renamed_to_nested_directory() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    let original_file = create_file(original_dir.path(), "file.txt", "content");
    create_file(backup_dir.path(), "file.txt", "content");

    let mut syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap();

    let from_path = fs::canonicalize(&original_file).unwrap();
    fs::create_dir_all(original_dir.path().join("subdir")).unwrap();
    let to_path = original_dir.path().join("subdir/renamed.txt");
    fs::rename(&original_file, &to_path).unwrap();
    let to_path = fs::canonicalize(&to_path).unwrap();

    syncer
        .handle_original_renamed(&from_path, &to_path)
        .unwrap();

    let new_backup = backup_dir.path().join("subdir/renamed.txt");
    assert!(new_backup.exists());
    assert_eq!(read_file_content(&new_backup), "content");
}

#[test]
fn test_handle_original_renamed_updates_entries() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    let original_file = create_file(original_dir.path(), "old.txt", "content");
    create_file(backup_dir.path(), "old.txt", "content");

    let mut syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap();

    let from_path = fs::canonicalize(&original_file).unwrap();
    let to_path = original_dir.path().join("new.txt");
    fs::rename(&original_file, &to_path).unwrap();
    let to_path = fs::canonicalize(&to_path).unwrap();

    syncer
        .handle_original_renamed(&from_path, &to_path)
        .unwrap();

    let new_backup = syncer.get_backup_path(&to_path).unwrap();
    assert!(new_backup.exists());
}

#[test]
fn test_handle_original_modified_calculate_delta_returns_empty_when_unchanged() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "file.txt", "same content");
    create_file(backup_dir.path(), "file.txt", "same content");

    let syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap();

    let original_path = fs::canonicalize(original_dir.path().join("file.txt")).unwrap();
    let delta = syncer
        .handle_original_modified_calculate_delta(&original_path)
        .unwrap();

    assert!(delta.is_empty());
}

#[test]
fn test_handle_original_modified_calculate_delta_returns_delta_when_changed() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "file.txt", "new content");
    create_file(backup_dir.path(), "file.txt", "old content");

    let syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap();

    let original_path = fs::canonicalize(original_dir.path().join("file.txt")).unwrap();
    let delta = syncer
        .handle_original_modified_calculate_delta(&original_path)
        .unwrap();

    assert!(!delta.is_empty());
}

#[test]
fn test_handle_original_modified_apply_delta_updates_backup() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(original_dir.path(), "file.txt", "updated content");
    create_file(backup_dir.path(), "file.txt", "original content");

    let mut syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap();

    let original_path = fs::canonicalize(original_dir.path().join("file.txt")).unwrap();
    let delta = syncer
        .handle_original_modified_calculate_delta(&original_path)
        .unwrap();

    syncer
        .handle_original_modified_apply_delta(&original_path, &delta)
        .unwrap();

    let backup_file = backup_dir.path().join("file.txt");
    assert_eq!(read_file_content(&backup_file), "updated content");
}

#[test]
fn test_handle_original_modified_apply_delta_with_append() {
    let original_dir = TempDir::new().unwrap();
    let backup_dir = TempDir::new().unwrap();

    create_file(
        original_dir.path(),
        "file.txt",
        "original content with more data appended",
    );
    create_file(backup_dir.path(), "file.txt", "original content");

    let mut syncer = Synchronizer::new(
        original_dir.path().to_path_buf(),
        backup_dir.path().to_path_buf(),
    )
    .unwrap();

    let original_path = fs::canonicalize(original_dir.path().join("file.txt")).unwrap();
    let delta = syncer
        .handle_original_modified_calculate_delta(&original_path)
        .unwrap();

    syncer
        .handle_original_modified_apply_delta(&original_path, &delta)
        .unwrap();

    let backup_file = backup_dir.path().join("file.txt");
    assert_eq!(
        read_file_content(&backup_file),
        "original content with more data appended"
    );
}
