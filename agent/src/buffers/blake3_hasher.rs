use anyhow::Context;
use blake3::{Hash, Hasher};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

pub struct Hashing<T> {
    inner: T,
    hasher: Hasher,
}

impl<T> Hashing<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            hasher: Hasher::new(),
        }
    }

    pub fn finalize(self) -> Hash {
        self.hasher.finalize()
    }
}

impl<T: Read> Hashing<T> {
    pub fn exhaust(&mut self) -> std::io::Result<Hash> {
        self.hasher.update_reader(&mut self.inner)?;
        Ok(self.hasher.finalize())
    }
}

impl<T: Read> Read for Hashing<T> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let bytes_read = self.inner.read(buf)?;
        if bytes_read > 0 {
            self.hasher.update(&buf[..bytes_read]);
        }
        Ok(bytes_read)
    }
}

impl<T: Write> Write for Hashing<T> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let bytes_written = self.inner.write(buf)?;
        if bytes_written > 0 {
            self.hasher.update(&buf[..bytes_written]);
        }
        Ok(bytes_written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

pub fn hash_file<T: AsRef<Path>>(reference: &T) -> anyhow::Result<Hash> {
    let file = fs::File::open(reference).context("Failed to open file")?;
    let mut reader = Hashing::new(file);
    reader.exhaust().with_context(|| {
        format!(
            "Failed to update hasher with reader: {}",
            reference.as_ref().display()
        )
    })
}
