use std::borrow::Borrow;
use std::collections::HashMap;
use std::fs::canonicalize;
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, RwLock};

static PATH_CACHE: LazyLock<Arc<RwLock<HashMap<PathBuf, PathBuf>>>> =
    LazyLock::new(|| Default::default());

/// Wrapper around [`std::fs::canonicalize`] that caches already canonicalized
/// entries globally (often the case when dealing with includes).
///
/// This is done due to the operation being inherently slow: It has to walk the
/// entire parent directory tree (especially expensive on FUSE-mounted virtual
/// repository filesystems).
pub fn canonicalize_cached<P>(path: P) -> Result<PathBuf, std::io::Error>
where
    P: AsRef<Path>,
    PathBuf: Borrow<P>,
    P: Hash + Eq,
{
    {
        // First, try the cache ...
        let cache = PATH_CACHE.read().unwrap();
        if let Some(cached) = cache.get(&path) {
            return Ok(cached.clone());
        }
    }

    // ... then look it up ourselves.
    let result = canonicalize(&path)?;
    let mut cache = PATH_CACHE.write().unwrap();
    cache.insert(path.as_ref().to_path_buf(), result.clone());

    Ok(result)
}
