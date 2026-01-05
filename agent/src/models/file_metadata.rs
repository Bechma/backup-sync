use serde::{Deserialize, Serialize};
use std::fs::Metadata;
use std::path::Path;
use thiserror::Error;
use time::OffsetDateTime;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

#[derive(Debug, Error)]
pub enum FileMetadataError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported file type")]
    UnsupportedFileType,
    #[error("timestamp error: {0}")]
    Timestamp(#[from] time::error::ComponentRange),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileType {
    File,
    Directory,
    Symlink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permissions {
    /// Unix mode bits (e.g., 0o755). On Windows, synthesized from attributes.
    mode: u32,
    /// Read-only flag (cross-platform)
    readonly: bool,
    /// Hidden file (Windows native, Unix: starts with dot)
    hidden: bool,
}

impl Permissions {
    #[must_use]
    pub fn new(mode: u32, readonly: bool, hidden: bool) -> Self {
        Self {
            mode: mode & 0o7777,
            readonly,
            hidden,
        }
    }

    #[must_use]
    pub fn from_mode(mode: u32) -> Self {
        Self {
            mode: mode & 0o7777,
            readonly: (mode & 0o200) == 0,
            hidden: false,
        }
    }

    #[must_use]
    pub fn default_file() -> Self {
        Self {
            mode: 0o644,
            readonly: false,
            hidden: false,
        }
    }

    #[must_use]
    pub fn default_directory() -> Self {
        Self {
            mode: 0o755,
            readonly: false,
            hidden: false,
        }
    }

    #[must_use]
    pub fn mode(self) -> u32 {
        self.mode
    }

    #[must_use]
    pub fn readonly(self) -> bool {
        self.readonly
    }

    #[must_use]
    pub fn hidden(self) -> bool {
        self.hidden
    }

    pub fn set_mode(&mut self, mode: u32) {
        self.mode = mode & 0o7777;
    }

    pub fn set_readonly(&mut self, readonly: bool) {
        self.readonly = readonly;
    }

    pub fn set_hidden(&mut self, hidden: bool) {
        self.hidden = hidden;
    }

    #[must_use]
    pub fn with_mode(mut self, mode: u32) -> Self {
        self.set_mode(mode);
        self
    }

    #[must_use]
    pub fn with_readonly(mut self, readonly: bool) -> Self {
        self.readonly = readonly;
        self
    }

    #[must_use]
    pub fn with_hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }
}

impl Default for Permissions {
    fn default() -> Self {
        Self::default_file()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMetadata {
    file_type: FileType,
    size: u64,
    permissions: Permissions,
    #[serde(with = "time::serde::rfc3339")]
    mtime: OffsetDateTime,
    // Linux only added creation time (btime) in kernel 4.11 via statx(), and many filesystems don't support it (ext3, older ext4, NFS, etc.).
    #[serde(default, with = "time::serde::rfc3339::option")]
    ctime: Option<OffsetDateTime>,
    symlink_target: Option<String>,
}

impl FileMetadata {
    #[must_use]
    pub fn new(
        file_type: FileType,
        size: u64,
        permissions: Permissions,
        mtime: OffsetDateTime,
    ) -> Self {
        Self {
            file_type,
            size,
            permissions,
            mtime,
            ctime: None,
            symlink_target: None,
        }
    }

    pub fn from_path(path: &Path) -> Result<Self, FileMetadataError> {
        let metadata = if path.is_symlink() {
            std::fs::symlink_metadata(path)?
        } else {
            std::fs::metadata(path)?
        };
        Self::from_std_metadata(&metadata, path)
    }

    pub fn from_std_metadata(metadata: &Metadata, path: &Path) -> Result<Self, FileMetadataError> {
        let file_type = if metadata.is_symlink() {
            FileType::Symlink
        } else if metadata.is_dir() {
            FileType::Directory
        } else if metadata.is_file() {
            FileType::File
        } else {
            return Err(FileMetadataError::UnsupportedFileType);
        };

        let permissions = Self::extract_permissions(metadata, path);
        let mtime = metadata
            .modified()
            .map_or_else(|_| Ok(OffsetDateTime::now_utc()), system_time_to_offset)?;
        let ctime = metadata
            .created()
            .ok()
            .map(system_time_to_offset)
            .transpose()?;

        let symlink_target = if file_type == FileType::Symlink {
            std::fs::read_link(path)
                .ok()
                .and_then(|p| p.to_str().map(String::from))
        } else {
            None
        };

        Ok(Self {
            file_type,
            size: metadata.len(),
            permissions,
            mtime,
            ctime,
            symlink_target,
        })
    }

    #[cfg(unix)]
    fn extract_permissions(metadata: &Metadata, path: &Path) -> Permissions {
        let mode = metadata.permissions().mode();
        let hidden = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with('.'));

        Permissions {
            mode: mode & 0o7777,
            readonly: metadata.permissions().readonly(),
            hidden,
        }
    }

    #[cfg(windows)]
    fn extract_permissions(metadata: &Metadata, _path: &Path) -> Permissions {
        let readonly = metadata.permissions().readonly();
        let attrs = metadata.file_attributes();

        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        let hidden = (attrs & FILE_ATTRIBUTE_HIDDEN) != 0;

        let mode = if metadata.is_dir() {
            if readonly { 0o555 } else { 0o755 }
        } else {
            if readonly { 0o444 } else { 0o644 }
        };

        Permissions {
            mode,
            readonly,
            hidden,
        }
    }

    #[cfg(not(any(unix, windows)))]
    fn extract_permissions(metadata: &Metadata, _path: &Path) -> Permissions {
        Permissions {
            mode: if metadata.is_dir() { 0o755 } else { 0o644 },
            readonly: metadata.permissions().readonly(),
            hidden: false,
        }
    }

    // Getters
    #[must_use]
    pub fn file_type(&self) -> FileType {
        self.file_type
    }

    #[must_use]
    pub fn size(&self) -> u64 {
        self.size
    }

    #[must_use]
    pub fn permissions(&self) -> &Permissions {
        &self.permissions
    }

    #[must_use]
    pub fn mtime(&self) -> OffsetDateTime {
        self.mtime
    }

    #[must_use]
    pub fn ctime(&self) -> Option<OffsetDateTime> {
        self.ctime
    }

    #[must_use]
    pub fn symlink_target(&self) -> Option<&str> {
        self.symlink_target.as_deref()
    }

    // Type checks
    #[must_use]
    pub fn is_file(&self) -> bool {
        self.file_type == FileType::File
    }

    #[must_use]
    pub fn is_dir(&self) -> bool {
        self.file_type == FileType::Directory
    }

    #[must_use]
    pub fn is_symlink(&self) -> bool {
        self.file_type == FileType::Symlink
    }

    // Apply metadata to filesystem
    pub fn apply_to(&self, path: &Path) -> Result<(), FileMetadataError> {
        self.apply_permissions(path)?;
        self.apply_times(path)?;
        Ok(())
    }

    #[cfg(unix)]
    fn apply_permissions(&self, path: &Path) -> Result<(), FileMetadataError> {
        use std::os::unix::fs::PermissionsExt;

        let perms = std::fs::Permissions::from_mode(self.permissions.mode);
        std::fs::set_permissions(path, perms)?;
        Ok(())
    }

    #[cfg(windows)]
    fn apply_permissions(&self, path: &Path) -> Result<(), FileMetadataError> {
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_readonly(self.permissions.readonly);
        std::fs::set_permissions(path, perms)?;

        if self.permissions.hidden {
            Self::set_hidden_attribute(path)?;
        }

        Ok(())
    }

    #[cfg(windows)]
    fn set_hidden_attribute(path: &Path) -> Result<(), FileMetadataError> {
        use std::os::windows::ffi::OsStrExt;

        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;

        let wide_path: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        unsafe {
            let attrs =
                windows_sys::Win32::Storage::FileSystem::GetFileAttributesW(wide_path.as_ptr());
            if attrs != u32::MAX {
                windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(
                    wide_path.as_ptr(),
                    attrs | FILE_ATTRIBUTE_HIDDEN,
                );
            }
        }

        Ok(())
    }

    #[cfg(not(any(unix, windows)))]
    fn apply_permissions(&self, path: &Path) -> Result<(), FileMetadataError> {
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_readonly(self.permissions.readonly);
        std::fs::set_permissions(path, perms)?;
        Ok(())
    }

    fn apply_times(&self, path: &Path) -> Result<(), FileMetadataError> {
        let mtime = filetime::FileTime::from_unix_time(
            self.mtime.unix_timestamp(),
            self.mtime.nanosecond(),
        );
        filetime::set_file_mtime(path, mtime)?;
        Ok(())
    }
}

fn system_time_to_offset(
    time: std::time::SystemTime,
) -> Result<OffsetDateTime, time::error::ComponentRange> {
    let duration = time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    OffsetDateTime::from_unix_timestamp(duration.as_secs().cast_signed())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_permissions_builder() {
        let perms = Permissions::default_file()
            .with_mode(0o755)
            .with_readonly(true)
            .with_hidden(true);

        assert_eq!(perms.mode(), 0o755);
        assert!(perms.readonly());
        assert!(perms.hidden());
    }

    #[test]
    fn test_read_file_metadata() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");

        let mut file = std::fs::File::create(&file_path).unwrap();
        file.write_all(b"hello world").unwrap();
        drop(file);

        let meta = FileMetadata::from_path(&file_path).unwrap();
        assert!(meta.is_file());
        assert_eq!(meta.size(), 11);
    }

    #[test]
    fn test_serde_roundtrip() {
        let meta = FileMetadata::new(
            FileType::File,
            1024,
            Permissions::default_file(),
            OffsetDateTime::now_utc(),
        );
        let json = postcard::to_allocvec(&meta).unwrap();
        let deserialized: FileMetadata = postcard::from_bytes(&json).unwrap();

        assert_eq!(meta.file_type(), deserialized.file_type());
        assert_eq!(meta.size(), deserialized.size());
    }

    #[cfg(unix)]
    #[test]
    fn test_apply_permissions_unix() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, b"test").unwrap();

        let meta = FileMetadata::new(
            FileType::File,
            4,
            Permissions::from_mode(0o755),
            OffsetDateTime::now_utc(),
        );
        meta.apply_permissions(&file_path).unwrap();

        let new_meta = std::fs::metadata(&file_path).unwrap();
        assert_eq!(new_meta.permissions().mode() & 0o777, 0o755);
    }
}
