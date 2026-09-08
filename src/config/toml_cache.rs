//! **Layer: Sync-Client (persistence utility)**
//!
//! TOML file cache and integrity helpers. Pure persistence functions with no
//! keyring, sync, or serde dependencies. Core modules (`library`) use
//! [`cached_read_toml`] and [`invalidate_toml_cache`] via the documented
//! carve-out; the sync/keychain parts of [`crate::config`] are not imported
//! by core.

use crate::error::{SnipError, SnipResult};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::Read;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::time::SystemTime;

pub(crate) struct CachedToml {
    pub(crate) mtime: SystemTime,
    pub(crate) len: u64,
    pub(crate) content: String,
    pub(crate) mtime_nanos: u32,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    device: u64,
    #[cfg(windows)]
    file_index: u64,
    #[cfg(windows)]
    volume_serial: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TomlMetadata {
    mtime: SystemTime,
    len: u64,
    mtime_nanos: u32,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    device: u64,
    #[cfg(windows)]
    file_index: u64,
    #[cfg(windows)]
    volume_serial: u64,
}

pub(crate) struct TomlCache {
    pub(crate) entries: HashMap<String, CachedToml>,
    pub(crate) insertion_order: VecDeque<String>,
}

const MAX_TOML_CACHE_SIZE: usize = 100;

pub(crate) static TOML_CACHE: LazyLock<Mutex<TomlCache>> = LazyLock::new(|| {
    Mutex::new(TomlCache {
        entries: HashMap::new(),
        insertion_order: VecDeque::new(),
    })
});

/// Lock the TOML cache. A poisoned mutex means a previous holder panicked
/// mid-update, so the cached contents may be inconsistent — recover by
/// clearing the cache rather than trusting it.
fn lock_toml_cache() -> std::sync::MutexGuard<'static, TomlCache> {
    match TOML_CACHE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            *guard = TomlCache {
                entries: HashMap::new(),
                insertion_order: VecDeque::new(),
            };
            guard
        }
    }
}

pub fn invalidate_toml_cache(path: &std::path::Path) {
    let key = toml_cache_key(path);
    let mut cache = lock_toml_cache();
    cache.entries.remove(&key);
    cache.insertion_order.retain(|k| k != &key);
    // BUG-02 mitigation: the cache key can diverge when a symlink is
    // atomically replaced (canonical path vs parent+filename fallback).
    // Remove the alternative form as well so a stale entry does not survive
    // an invalidation done via the other string form.
    let alt_key = path
        .parent()
        .and_then(|parent| parent.canonicalize().ok())
        .map(|cp| cp.join(path.file_name().unwrap_or_default()))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !alt_key.is_empty() && alt_key != key {
        cache.entries.remove(&alt_key);
        cache.insertion_order.retain(|k| k != &alt_key);
    }
    let raw_key = path.to_path_buf().to_string_lossy().into_owned();
    if raw_key != key && raw_key != alt_key {
        cache.entries.remove(&raw_key);
        cache.insertion_order.retain(|k| k != &raw_key);
    }
}

pub(crate) fn toml_cache_key(path: &std::path::Path) -> String {
    if let Ok(canonical) = path.canonicalize() {
        return canonical.to_string_lossy().into_owned();
    }
    if let Some(parent) = path.parent()
        && let Ok(canonical_parent) = parent.canonicalize()
    {
        let candidate = canonical_parent.join(path.file_name().unwrap_or_default());
        if let Ok(canonical_candidate) = candidate.canonicalize() {
            return canonical_candidate.to_string_lossy().into_owned();
        }
        return candidate.to_string_lossy().into_owned();
    }
    path.to_path_buf().to_string_lossy().into_owned()
}

fn toml_metadata(file: &fs::File, path: &std::path::Path) -> SnipResult<TomlMetadata> {
    let metadata = file
        .metadata()
        .map_err(|e| SnipError::io_error("stat toml file", path.to_path_buf(), e))?;
    let mtime = metadata
        .modified()
        .map_err(|e| SnipError::io_error("read mtime", path.to_path_buf(), e))?;
    let mtime_nanos = mtime
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    #[cfg(windows)]
    let (file_index, volume_serial) = windows_file_identity(file).unwrap_or((0, 0));

    Ok(TomlMetadata {
        mtime,
        len: metadata.len(),
        mtime_nanos,
        #[cfg(unix)]
        inode: {
            use std::os::unix::fs::MetadataExt;
            metadata.ino()
        },
        #[cfg(unix)]
        device: {
            use std::os::unix::fs::MetadataExt;
            metadata.dev()
        },
        #[cfg(windows)]
        file_index,
        #[cfg(windows)]
        volume_serial,
    })
}

#[cfg(windows)]
fn windows_file_identity(file: &fs::File) -> Option<(u64, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };

    let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) };
    if ok == 0 {
        return None;
    }

    let info = unsafe { info.assume_init() };
    let file_index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    Some((file_index, u64::from(info.dwVolumeSerialNumber)))
}

fn toml_path_metadata(path: &std::path::Path) -> SnipResult<TomlMetadata> {
    let file = fs::File::open(path)
        .map_err(|e| SnipError::io_error("open toml file", path.to_path_buf(), e))?;
    toml_metadata(&file, path)
}

pub(crate) fn compute_crc32(data: &str) -> u32 {
    crc32fast::hash(data.as_bytes())
}

pub(crate) fn split_integrity_header(content: &str) -> Option<(&str, &str)> {
    let (first_line, body) = match content.find('\n') {
        Some(index) => (&content[..index], &content[index + 1..]),
        None => (content, ""),
    };

    first_line
        .strip_prefix("# integrity:")
        .map(|checksum| (checksum.trim(), body))
}

/// Verifies CRC32 integrity of the config file content.
///
/// Note: CRC32 detects accidental corruption (e.g., partial writes, disk errors)
/// but is NOT a cryptographic integrity check. An attacker who can modify the
/// config file can recalculate the CRC32. This is acceptable because the threat
/// model assumes local-only access — if an attacker can write to the config
/// directory, they can already replace the entire file or binary.
pub(crate) fn verify_integrity(content: &str) -> bool {
    // The integrity header must be the very first line to avoid matching
    // user-authored TOML comments like "# integrity: 42".
    if let Some((checksum, body)) = split_integrity_header(content) {
        return checksum
            .parse::<u32>()
            .is_ok_and(|stored| stored == compute_crc32(body));
    }

    // No integrity header found — this is a legacy config file from before the
    // integrity feature was added. Treat it as valid rather than silently
    // replacing with defaults (which would cause data loss on upgrade).
    // The header will be added on the next save.
    true
}

pub(crate) fn strip_integrity_line(content: &str) -> String {
    split_integrity_header(content)
        .map(|(_, body)| body.to_string())
        .unwrap_or_else(|| content.to_string())
}

pub fn cached_read_toml(path: &std::path::Path) -> SnipResult<String> {
    let key = toml_cache_key(path);

    let cache = lock_toml_cache();
    let path_metadata = toml_path_metadata(path)?;
    if let Some(entry) = cache.entries.get(&key)
        && entry.mtime == path_metadata.mtime
        && entry.mtime_nanos == path_metadata.mtime_nanos
        && entry.len == path_metadata.len
        && {
            #[cfg(unix)]
            {
                entry.inode == path_metadata.inode && entry.device == path_metadata.device
            }
            #[cfg(windows)]
            {
                entry.file_index == path_metadata.file_index
                    && entry.volume_serial == path_metadata.volume_serial
            }
            #[cfg(not(any(unix, windows)))]
            {
                true
            }
        }
    {
        return Ok(entry.content.clone());
    }
    drop(cache);

    // Read through the opened file and verify that its metadata did not change
    // during the read. This keeps content and cache metadata tied to one file
    // snapshot, even if the path is atomically replaced concurrently.
    for _ in 0..2 {
        let mut file = fs::File::open(path)
            .map_err(|e| SnipError::io_error("open toml file", path.to_path_buf(), e))?;
        let before = toml_metadata(&file, path)?;
        let mut content = String::new();
        file.read_to_string(&mut content)
            .map_err(|e| SnipError::io_error("read toml file", path.to_path_buf(), e))?;
        let after = toml_metadata(&file, path)?;
        if before != after {
            continue;
        }

        let mut cache = lock_toml_cache();
        while cache.entries.len() >= MAX_TOML_CACHE_SIZE {
            let Some(oldest) = cache.insertion_order.pop_front() else {
                break;
            };
            if cache.entries.remove(&oldest).is_some() {
                break;
            }
        }

        if !cache.entries.contains_key(&key) {
            cache.insertion_order.push_back(key.clone());
        }
        cache.entries.insert(
            key.clone(),
            CachedToml {
                mtime: after.mtime,
                len: after.len,
                content: content.clone(),
                mtime_nanos: after.mtime_nanos,
                #[cfg(unix)]
                inode: after.inode,
                #[cfg(unix)]
                device: after.device,
                #[cfg(windows)]
                file_index: after.file_index,
                #[cfg(windows)]
                volume_serial: after.volume_serial,
            },
        );
        return Ok(content);
    }

    Err(SnipError::runtime_error(
        "read toml file",
        Some("file changed while being read"),
    ))
}
