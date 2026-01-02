use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Error)]
pub enum RelativePathError {
    #[error("path cannot be empty")]
    Empty,
    #[error("path cannot be absolute: {0}")]
    Absolute(String),
    #[error("path cannot contain parent directory references (..)")]
    ParentTraversal,
    #[error("path contains invalid characters")]
    InvalidCharacters,
    #[error("path is too long (max {max} bytes, got {got})")]
    TooLong { max: usize, got: usize },
    #[error("invalid UTF-8 in path")]
    InvalidUtf8,
}

const MAX_PATH_LENGTH: usize = 4096;
const FORBIDDEN_CHARS: &[char] = &['\0'];
const WINDOWS_FORBIDDEN: &[char] = &['<', '>', ':', '"', '|', '?', '*'];

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RelativePath(String);

impl RelativePath {
    pub fn new(path: impl AsRef<str>) -> Result<Self, RelativePathError> {
        let path = path.as_ref();

        if path.is_empty() {
            return Err(RelativePathError::Empty);
        }

        if path.len() > MAX_PATH_LENGTH {
            return Err(RelativePathError::TooLong {
                max: MAX_PATH_LENGTH,
                got: path.len(),
            });
        }

        // Normalize Unicode to NFC (composed form) for cross-platform consistency
        let path: String = path.nfc().collect();

        // Normalize: convert backslashes to forward slashes
        let normalized: String = path.replace('\\', "/");

        // Check for absolute paths (Unix and Windows styles)
        if normalized.starts_with('/') || normalized.chars().nth(1) == Some(':') {
            return Err(RelativePathError::Absolute(path.clone()));
        }

        // Check for truly forbidden characters (null bytes, control chars)
        if normalized
            .chars()
            .any(|c| FORBIDDEN_CHARS.contains(&c) || (c.is_control() && c != '\t'))
        {
            return Err(RelativePathError::InvalidCharacters);
        }

        // Normalize and validate components
        let mut clean_components: Vec<&str> = Vec::new();

        for component in normalized.split('/') {
            match component {
                "" | "." => {}
                ".." => return Err(RelativePathError::ParentTraversal),
                c => {
                    clean_components.push(c);
                }
            }
        }

        if clean_components.is_empty() {
            return Err(RelativePathError::Empty);
        }

        Ok(Self(clean_components.join("/")))
    }

    /// Create from a native Path, relative to a base directory
    pub fn from_path(path: &Path, base: &Path) -> Result<Self, RelativePathError> {
        let relative = path
            .strip_prefix(base)
            .map_err(|_| RelativePathError::Absolute(path.display().to_string()))?;

        let s = relative.to_str().ok_or(RelativePathError::InvalidUtf8)?;

        Self::new(s)
    }

    /// Convert to native `PathBuf`, sanitizing for the current OS
    #[must_use] 
    pub fn to_path_buf(&self) -> PathBuf {
        PathBuf::from(self.to_native_string())
    }

    /// Convert to native path string, sanitizing for the current OS
    #[must_use] 
    pub fn to_native_string(&self) -> String {
        #[cfg(windows)]
        {
            self.to_windows_safe()
        }
        #[cfg(not(windows))]
        {
            self.0.clone()
        }
    }

    /// Convert to Windows-safe path string (replaces forbidden chars with _)
    pub fn to_windows_safe(&self) -> String {
        let mut result = String::with_capacity(self.0.len());

        for c in self.0.chars() {
            if c == '/' {
                result.push('\\');
            } else if WINDOWS_FORBIDDEN.contains(&c) {
                result.push('_');
            } else {
                result.push(c);
            }
        }

        // Handle Windows reserved names in each component
        result
            .split('\\')
            .map(sanitize_windows_component)
            .collect::<Vec<_>>()
            .join("\\")
    }

    /// Resolve against a base directory (uses native path format)
    #[must_use] 
    pub fn resolve(&self, base: &Path) -> PathBuf {
        base.join(self.to_path_buf())
    }

    /// Check if this path contains Windows-incompatible characters
    pub fn has_windows_incompatible_chars(&self) -> bool {
        self.0.chars().any(|c| WINDOWS_FORBIDDEN.contains(&c))
            || self.0.split('/').any(is_windows_reserved)
    }

    /// Get the file name component
    #[must_use] 
    pub fn file_name(&self) -> Option<&str> {
        self.0.rsplit('/').next()
    }

    /// Join with another relative path component
    pub fn join(&self, other: &str) -> Result<RelativePath, RelativePathError> {
        RelativePath::new(format!("{}/{}", self.0, other))
    }
}

fn is_windows_reserved(name: &str) -> bool {
    let base = name.split('.').next().unwrap_or(name).to_uppercase();
    matches!(
        base.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn sanitize_windows_component(component: &str) -> String {
    let base = component.split('.').next().unwrap_or(component);

    if is_windows_reserved(base) {
        // Prefix with underscore: CON -> _CON
        format!("_{component}")
    } else if component.ends_with('.') || component.ends_with(' ') {
        // Trailing dots/spaces are problematic on Windows
        format!("{}_", component.trim_end_matches(['.', ' ']))
    } else {
        component.to_string()
    }
}

// Serde support
impl TryFrom<String> for RelativePath {
    type Error = RelativePathError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        RelativePath::new(value)
    }
}

impl From<RelativePath> for String {
    fn from(path: RelativePath) -> Self {
        path.0
    }
}

impl TryFrom<PathBuf> for RelativePath {
    type Error = RelativePathError;

    fn try_from(path: PathBuf) -> Result<Self, Self::Error> {
        Self::new(path.to_str().ok_or(RelativePathError::InvalidUtf8)?)
    }
}

impl std::fmt::Display for RelativePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for RelativePath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl PartialOrd for RelativePath {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RelativePath {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_windows_forbidden_chars_allowed() {
        // These are now allowed in the canonical representation
        assert!(RelativePath::new("file:name.txt").is_ok());
        assert!(RelativePath::new("what?.txt").is_ok());
        assert!(RelativePath::new("file<>name.txt").is_ok());
    }

    #[test]
    fn test_windows_safe_conversion() {
        let p = RelativePath::new("file:name?.txt").unwrap();
        assert_eq!(p.as_ref(), "file:name?.txt"); // Original preserved
        assert_eq!(p.to_windows_safe(), "file_name_.txt"); // Sanitized for Windows
    }

    #[test]
    fn test_windows_reserved_names() {
        let p = RelativePath::new("CON").unwrap();
        assert_eq!(p.to_windows_safe(), "_CON");

        let p = RelativePath::new("dir/NUL.txt").unwrap();
        assert_eq!(p.to_windows_safe(), "dir\\_NUL.txt");
    }

    #[test]
    fn test_has_windows_incompatible() {
        let p = RelativePath::new("normal.txt").unwrap();
        assert!(!p.has_windows_incompatible_chars());

        let p = RelativePath::new("file:name.txt").unwrap();
        assert!(p.has_windows_incompatible_chars());

        let p = RelativePath::new("CON").unwrap();
        assert!(p.has_windows_incompatible_chars());
    }

    #[test]
    fn test_unicode_normalization() {
        // e + combining acute accent (NFD) should normalize to é (NFC)
        let nfd = "cafe\u{0301}"; // NFD form
        let p = RelativePath::new(nfd).unwrap();
        assert_eq!(p.as_ref(), "café"); // NFC form
    }

    #[test]
    fn test_still_rejects_truly_invalid() {
        assert!(RelativePath::new("").is_err());
        assert!(RelativePath::new("/absolute").is_err());
        assert!(RelativePath::new("../escape").is_err());
        assert!(RelativePath::new("file\0name").is_err()); // Null byte
    }

    #[test]
    fn test_native_path_conversion() {
        let p = RelativePath::new("dir/file:name.txt").unwrap();

        #[cfg(windows)]
        assert_eq!(p.to_native_string(), "dir\\file_name.txt");

        #[cfg(not(windows))]
        assert_eq!(p.to_native_string(), "dir/file:name.txt");
    }
}
