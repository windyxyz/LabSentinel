//! Per-user, disposable on-disk snapshots of locally generated Vulkan pipelines.

use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
};

pub(super) const LIMIT: usize = 16 * 1024 * 1024;
const SEALED_LIMIT: usize = LIMIT + 16 * 1024;
const ENTROPY: &[u8] = b"LabSentinel Vulkan pipeline cache v1";

pub(super) fn fingerprint(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

struct ProtectedBuffer(CRYPT_INTEGER_BLOB);

impl Drop for ProtectedBuffer {
    fn drop(&mut self) {
        if !self.0.pbData.is_null() {
            unsafe { LocalFree(self.0.pbData.cast()) };
        }
    }
}

fn transform(input: &[u8], restoring: bool) -> io::Result<Vec<u8>> {
    let blob = |bytes: &[u8]| CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let data = blob(input);
    let entropy = blob(ENTROPY);
    let mut out = ProtectedBuffer(CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    });
    // Input slices outlive these synchronous calls. DPAPI's allocated output is
    // copied before Windows frees it through ProtectedBuffer.
    let ok = unsafe {
        if restoring {
            CryptUnprotectData(
                &data,
                null_mut(),
                &entropy,
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out.0,
            )
        } else {
            CryptProtectData(
                &data,
                null(),
                &entropy,
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out.0,
            )
        }
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let size = out.0.cbData as usize;
    let limit = if restoring { LIMIT + 32 } else { SEALED_LIMIT };
    if out.0.pbData.is_null() || !(1..=limit).contains(&size) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "pipeline cache blob size",
        ));
    }
    Ok(unsafe { std::slice::from_raw_parts(out.0.pbData, size) }.to_vec())
}

pub(super) fn read(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let meta = file.metadata()?;
    if !meta.is_file() || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "pipeline cache must be a regular file",
        ));
    }
    let mut sealed = Vec::new();
    file.take((SEALED_LIMIT + 1) as u64)
        .read_to_end(&mut sealed)?;
    if !(1..=SEALED_LIMIT).contains(&sealed.len()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "pipeline cache file size",
        ));
    }
    let clear = transform(&sealed, true)?;
    let Some((digest, data)) = clear.split_at_checked(32) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "pipeline cache digest is missing",
        ));
    };
    if data.is_empty() || digest != fingerprint(data) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "pipeline cache checksum mismatch",
        ));
    }
    Ok(Some(data.to_vec()))
}

pub(super) fn write(path: &Path, data: &[u8], digest: [u8; 32]) -> io::Result<()> {
    if !(1..=LIMIT).contains(&data.len()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "pipeline cache payload size",
        ));
    }
    if let Ok(meta) = fs::symlink_metadata(path) {
        if !meta.is_file() || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "pipeline cache must be a regular file",
            ));
        }
    }
    let mut clear = Vec::with_capacity(32 + data.len());
    clear.extend_from_slice(&digest);
    clear.extend_from_slice(data);
    let sealed = transform(&clear, false)?;
    let mut temporary = tempfile::NamedTempFile::new_in(path.parent().expect("cache directory"))?;
    temporary.write_all(&sealed)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_cache_round_trips_and_rejects_tampering() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("pipeline-cache.bin");
        assert!(read(&file).unwrap().is_none());
        let data = b"locally compiled pipeline";
        write(&file, data, fingerprint(data)).unwrap();
        assert_eq!(read(&file).unwrap().unwrap(), data);
        let mut sealed = fs::read(&file).unwrap();
        let midpoint = sealed.len() / 2;
        sealed[midpoint] ^= 1;
        fs::write(&file, sealed).unwrap();
        assert!(read(&file).is_err());
        write(&file, data, [0; 32]).unwrap();
        assert!(read(&file).is_err());
        write(&file, data, fingerprint(data)).unwrap();
        assert_eq!(read(&file).unwrap().unwrap(), data);
    }
}
