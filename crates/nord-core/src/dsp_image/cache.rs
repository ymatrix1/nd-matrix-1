//! One built image per OS update file path, read once (setup time only, D128).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

type Entry<T> = Option<(PathBuf, Result<Arc<T>, String>)>;

/// Keeps the image built from the last named OS update file (or the reason it could not be built).
pub struct ImageCache<T>(Mutex<Entry<T>>);

impl<T> ImageCache<T> {
    pub const fn new() -> Self {
        Self(Mutex::new(None))
    }

    /// The image of the file named by `value` (the `NS4_WAVE_TABLES` text): `Ok(None)` when no
    /// file is named (`None` or blank), an error when it cannot be read or `build` rejects it.
    /// A repeated call with the same path returns the kept result without reading the file.
    pub fn get(
        &self,
        value: Option<&str>,
        build: impl FnOnce(&Path) -> Result<T, String>,
    ) -> Result<Option<Arc<T>>, String> {
        let Some(path) = value.map(str::trim).filter(|v| !v.is_empty()) else {
            return Ok(None);
        };
        let path = PathBuf::from(path);
        let mut cache = self
            .0
            .lock()
            .map_err(|_| "image cache poisoned".to_string())?;
        if let Some((p, r)) = cache.as_ref() {
            if *p == path {
                return r.clone().map(Some);
            }
        }
        let r = build(&path).map(Arc::new);
        *cache = Some((path, r.clone()));
        r.map(Some)
    }
}

impl<T> Default for ImageCache<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn an_unset_or_blank_value_names_no_file() {
        let c: ImageCache<u8> = ImageCache::new();
        assert_eq!(c.get(None, |_| Ok(1)), Ok(None));
        assert_eq!(c.get(Some("  "), |_| Ok(1)), Ok(None));
    }

    #[test]
    fn a_path_is_built_once_and_a_failure_is_kept_too() {
        let c: ImageCache<u8> = ImageCache::new();
        let calls = Cell::new(0);
        let build = |_: &Path| {
            calls.set(calls.get() + 1);
            Ok(7)
        };
        assert_eq!(*c.get(Some("a.exe"), build).unwrap().unwrap(), 7);
        assert_eq!(*c.get(Some(" a.exe "), build).unwrap().unwrap(), 7);
        assert_eq!(calls.get(), 1, "same path: kept");
        let e = c.get(Some("b.exe"), |_| Err("bad".into())).unwrap_err();
        assert_eq!(e, "bad");
        assert_eq!(c.get(Some("b.exe"), |_| Ok(1)), Err("bad".into()), "kept");
        assert_eq!(*c.get(Some("a.exe"), build).unwrap().unwrap(), 7);
        assert_eq!(calls.get(), 2, "another path replaces the entry");
    }
}
