use librsync::whole::signature;
use std::fs::File;
use std::io::{Result, Seek};

pub(crate) fn create_signature(f: &mut File) -> Result<Vec<u8>> {
    let mut sig = Vec::<u8>::new();
    signature(f, &mut sig).map_err(std::io::Error::other)?;
    f.seek(std::io::SeekFrom::Start(0))?;
    Ok(sig)
}
