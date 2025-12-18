use std::fs::{self, File};
use std::io::{Seek, Write};
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use fs2::FileExt;
use librsync::whole::{delta, patch, signature};

pub struct LocalFileOps;

impl LocalFileOps {
    fn open_for_read(path: &Path) -> Result<File> {
        File::open(path).with_context(|| format!("Failed to open file for reading: {path:?}"))
    }

    pub fn handle_original_modified_apply_delta(backup_path: &Path, dlt: &[u8]) -> Result<()> {
        let mut old_file = LocalFileOps::open_for_read_write(backup_path)?;

        let out = LocalFileOps::apply_patch(&mut old_file, dlt, backup_path)?;
        LocalFileOps::truncate_and_write(&mut old_file, &out, backup_path)
    }

    fn open_for_read_write(path: &Path) -> Result<File> {
        File::options()
            .read(true)
            .write(true)
            .open(path)
            .with_context(|| format!("Failed to open file for read/write: {path:?}"))
    }

    pub fn create_dir_all(path: &Path) -> Result<()> {
        fs::create_dir_all(path).with_context(|| format!("Failed to create directory: {path:?}"))
    }

    pub fn copy_file(from: &Path, to: &Path) -> Result<u64> {
        if let Some(parent) = to.parent() {
            Self::create_dir_all(parent)?;
        }
        fs::copy(from, to).with_context(|| format!("Failed to copy {from:?} to {to:?}"))
    }

    pub fn rename_file(from: &Path, to: &Path) -> Result<()> {
        if !from.exists() {
            return Err(anyhow!(
                "Failed to rename {from:?} to {to:?}: {from:?} does not exist"
            ));
        }
        if let Some(parent) = to.parent() {
            Self::create_dir_all(parent)?;
        }
        fs::rename(from, to).with_context(|| format!("Failed to rename {from:?} to {to:?}"))
    }

    pub fn remove_file(path: &Path) -> Result<()> {
        if !path.exists() {
            return Ok(());
        }
        fs::remove_file(path).with_context(|| format!("Failed to remove file: {path:?}"))
    }

    pub fn remove_dir_all(path: &Path) -> Result<()> {
        fs::remove_dir_all(path).with_context(|| format!("Failed to remove directory: {path:?}"))
    }

    pub fn lock_shared(path: &Path) -> Result<File> {
        let file = Self::open_for_read(path)?;
        file.lock_shared()
            .with_context(|| format!("Failed to acquire shared lock on: {path:?}"))?;
        Ok(file)
    }

    pub fn lock_exclusive(path: &Path) -> Result<File> {
        let file = Self::open_for_read_write(path)?;
        file.lock_exclusive()
            .with_context(|| format!("Failed to acquire exclusive lock on: {path:?}"))?;
        Ok(file)
    }

    fn truncate_and_write(file: &mut File, data: &[u8], path: &Path) -> Result<()> {
        file.set_len(0)
            .with_context(|| format!("Failed to truncate file: {path:?}"))?;
        file.write_all(data)
            .with_context(|| format!("Failed to write to file: {path:?}"))?;
        file.sync_data()
            .with_context(|| format!("Failed to sync file: {path:?}"))
    }

    pub fn create_signature(path: &Path) -> Result<Vec<u8>> {
        let mut file = Self::open_for_read(path)?;
        let mut sig = Vec::<u8>::new();
        signature(&mut file, &mut sig)
            .map_err(std::io::Error::other)
            .with_context(|| format!("Failed to create signature for: {path:?}"))?;
        Ok(sig)
    }

    pub fn calculate_delta(old_sig: &[u8], path: &Path) -> Result<Vec<u8>> {
        let mut new_file = Self::open_for_read(path)?;
        new_file
            .seek(std::io::SeekFrom::Start(0))
            .with_context(|| format!("Failed to seek file: {path:?}"))?;
        let mut dlt = Vec::<u8>::new();
        let mut sig_reader = old_sig;
        delta(&mut new_file, &mut sig_reader, &mut dlt)
            .map_err(std::io::Error::other)
            .with_context(|| format!("Failed to calculate delta for: {path:?}"))?;
        Ok(dlt)
    }

    fn apply_patch(old_file: &mut File, dlt: &[u8], path: &Path) -> Result<Vec<u8>> {
        old_file
            .seek(std::io::SeekFrom::Start(0))
            .with_context(|| format!("Failed to seek file: {path:?}"))?;
        let mut out = Vec::<u8>::new();
        let mut dlt_reader = dlt;
        patch(old_file, &mut dlt_reader, &mut out)
            .map_err(std::io::Error::other)
            .with_context(|| format!("Failed to apply patch to: {path:?}"))?;
        Ok(out)
    }
}
