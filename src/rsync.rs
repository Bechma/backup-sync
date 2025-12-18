use librsync::whole::{delta, patch, signature};
use std::fs::File;
use std::io::{Result, Seek};

pub(crate) fn create_signature(f: &mut File) -> Result<Vec<u8>> {
    f.seek(std::io::SeekFrom::Start(0))?;
    let mut sig = Vec::<u8>::new();
    signature(f, &mut sig).map_err(std::io::Error::other)?;
    Ok(sig)
}

pub(crate) fn calculate_delta(new_file: &mut File, mut old_sig: &[u8]) -> Result<Vec<u8>> {
    new_file.seek(std::io::SeekFrom::Start(0))?;
    let mut dlt = Vec::<u8>::new();
    delta(new_file, &mut old_sig, &mut dlt).map_err(std::io::Error::other)?;
    Ok(dlt)
}

pub(crate) fn apply_patch(old_file: &mut File, mut dlt: &[u8]) -> Result<Vec<u8>> {
    old_file.seek(std::io::SeekFrom::Start(0))?;
    let mut out = Vec::<u8>::new();
    patch(old_file, &mut dlt, &mut out).map_err(std::io::Error::other)?;
    Ok(out)
}
