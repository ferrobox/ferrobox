//! `StoragePort` adapter that keeps artifact bytes in a directory.
//!
//! Forge, Mirror, and Alloy do not change. This is another place to
//! put the same bytes, instead of an S3 bucket. A key is a relative
//! path under one root. `..`, an absolute path, a NUL byte, and an
//! empty key are rejected. Writes land in a sibling temporary file
//! and then rename, so a reader does not observe a half-written object.

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use bytes::Bytes;
use ferrobox_ports::storage::{StorageError, StorageKey, StoragePort};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// [`StoragePort`] adapter rooted at one directory.
pub struct FsStorageAdapter {
    root: PathBuf,
}

impl FsStorageAdapter {
    /// Creates `root` when it is missing and remembers its canonical path.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Backend`] when the directory cannot be
    /// created or canonicalized.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, StorageError> {
        let root = root.as_ref();
        std::fs::create_dir_all(root).map_err(io_error)?;
        let root = root.canonicalize().map_err(io_error)?;
        Ok(Self { root })
    }

    /// Canonical directory that holds every object.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn object_path(&self, key: &StorageKey) -> Result<PathBuf, StorageError> {
        Ok(self.root.join(relative_key(key)?))
    }

    async fn prepare_parent(&self, path: &Path) -> Result<(), StorageError> {
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| backend_message("resolved path escaped the storage directory"))?;
        let Some(parent) = relative.parent() else {
            return Err(backend_message("object has no parent directory"));
        };
        let mut current = self.root.clone();
        for component in parent.components() {
            current.push(component);
            match tokio::fs::symlink_metadata(&current).await {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(backend_message(
                        "refusing to follow a symlink inside the storage directory",
                    ));
                }
                Ok(meta) if meta.is_dir() => {}
                Ok(_) => {
                    return Err(backend_message(
                        "a parent of the object path is not a directory",
                    ));
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    tokio::fs::create_dir(&current).await.map_err(io_error)?;
                }
                Err(err) => return Err(io_error(err)),
            }
        }
        Ok(())
    }
}

#[async_trait]
impl StoragePort for FsStorageAdapter {
    async fn put(&self, key: &StorageKey, content: Bytes) -> Result<(), StorageError> {
        let path = self.object_path(key)?;
        self.prepare_parent(&path).await?;
        let tmp = temp_path(&path)?;
        if let Err(err) = tokio::fs::write(&tmp, &content).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(io_error(err));
        }
        if let Err(err) = tokio::fs::rename(&tmp, &path).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(io_error(err));
        }
        Ok(())
    }

    async fn get(&self, key: &StorageKey) -> Result<Bytes, StorageError> {
        let path = self.object_path(key)?;
        match file_metadata(&path).await? {
            None => Err(StorageError::NotFound(key.clone())),
            Some(()) => tokio::fs::read(&path)
                .await
                .map(Bytes::from)
                .map_err(io_error),
        }
    }

    async fn delete(&self, key: &StorageKey) -> Result<(), StorageError> {
        let path = self.object_path(key)?;
        match tokio::fs::symlink_metadata(&path).await {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(io_error(err)),
            Ok(meta) if meta.is_dir() => Err(backend_message("object path is a directory")),
            Ok(_) => tokio::fs::remove_file(&path).await.map_err(io_error),
        }
    }

    async fn exists(&self, key: &StorageKey) -> Result<bool, StorageError> {
        let path = self.object_path(key)?;
        file_metadata(&path).await.map(|found| found.is_some())
    }
}

fn relative_key(key: &StorageKey) -> Result<PathBuf, StorageError> {
    let raw = key.as_str();
    if raw.is_empty() || raw.contains('\0') || raw.starts_with('/') || raw.starts_with('\\') {
        return Err(invalid_key());
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(invalid_key());
    }
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(invalid_key());
            }
        }
    }
    if relative.as_os_str().is_empty() {
        return Err(invalid_key());
    }
    Ok(relative)
}

/// `Some` when `path` is a regular file. `None` when it is missing.
/// A symlink is refused so a key cannot read outside the root.
async fn file_metadata(path: &Path) -> Result<Option<()>, StorageError> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(meta) if meta.file_type().is_symlink() => Err(backend_message(
            "refusing to follow a symlink inside the storage directory",
        )),
        Ok(meta) if meta.is_file() => Ok(Some(())),
        Ok(_) => Err(backend_message("object path is not a file")),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(io_error(err)),
    }
}

fn temp_path(path: &Path) -> Result<PathBuf, StorageError> {
    let name = path
        .file_name()
        .ok_or_else(|| backend_message("object path has no file name"))?;
    let parent = path
        .parent()
        .ok_or_else(|| backend_message("object path has no parent"))?;
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| backend_message("system clock is before the unix epoch"))?
        .as_nanos();
    let tmp_name = format!(
        ".{}.ferrobox-tmp-{}-{}-{}",
        name.to_string_lossy(),
        std::process::id(),
        nanos,
        seq
    );
    Ok(parent.join(tmp_name))
}

fn invalid_key() -> StorageError {
    backend_message("invalid storage key")
}

fn backend_message(message: &'static str) -> StorageError {
    StorageError::Backend(Box::new(std::io::Error::other(message)))
}

fn io_error(err: std::io::Error) -> StorageError {
    StorageError::Backend(Box::new(err))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static TEMP_DIRS: AtomicU64 = AtomicU64::new(0);

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let n = TEMP_DIRS.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("ferrobox-fs-{n}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn put_get_delete_and_missing() {
        let dir = TempRoot::new();
        let storage = FsStorageAdapter::open(&dir.0).unwrap();
        let key = StorageKey::new("notes/readme.txt");

        storage
            .put(&key, Bytes::from_static(b"hello"))
            .await
            .unwrap();
        assert_eq!(
            storage.get(&key).await.unwrap(),
            Bytes::from_static(b"hello")
        );
        assert!(storage.exists(&key).await.unwrap());

        storage
            .put(&key, Bytes::from_static(b"again"))
            .await
            .unwrap();
        assert_eq!(
            storage.get(&key).await.unwrap(),
            Bytes::from_static(b"again")
        );

        let file = storage.root().join("notes/readme.txt");
        assert!(file.is_file());
        assert!(file.starts_with(storage.root()));

        storage.delete(&key).await.unwrap();
        assert!(!storage.exists(&key).await.unwrap());
        storage.delete(&key).await.unwrap();

        let missing = storage.get(&key).await.unwrap_err();
        assert!(matches!(missing, StorageError::NotFound(_)));
    }

    #[tokio::test]
    async fn rejects_keys_that_escape_the_root() {
        let dir = TempRoot::new();
        let storage = FsStorageAdapter::open(&dir.0).unwrap();
        for raw in [
            "",
            "..",
            "../etc/passwd",
            "/etc/passwd",
            "foo/../../etc/passwd",
            "a\0b",
            ".",
        ] {
            let key = StorageKey::new(raw);
            let err = storage
                .put(&key, Bytes::from_static(b"no"))
                .await
                .unwrap_err();
            assert!(
                matches!(err, StorageError::Backend(_)),
                "key {raw:?} should be rejected, got {err}"
            );
            assert!(
                storage.get(&key).await.is_err(),
                "key {raw:?} should not be readable"
            );
        }
        assert!(std::fs::read_dir(&dir.0).unwrap().next().is_none());
    }

    #[tokio::test]
    async fn refuses_a_symlink_inside_the_root() {
        let dir = TempRoot::new();
        let outside = TempRoot::new();
        std::fs::write(outside.0.join("secret"), b"hidden").unwrap();
        std::os::unix::fs::symlink(outside.0.join("secret"), dir.0.join("leak")).unwrap();

        let storage = FsStorageAdapter::open(&dir.0).unwrap();
        let key = StorageKey::new("leak");
        assert!(matches!(
            storage.get(&key).await.unwrap_err(),
            StorageError::Backend(_)
        ));
        assert!(storage.exists(&key).await.is_err());
    }
}
