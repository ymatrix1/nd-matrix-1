//! What a host project remembers about the sound library (D262): the `.ns4b` path with a cheap
//! identity check, and which parameter groups the user had edited on top of the loaded program.
//!
//! Before D262 the project saved only the `library_program` parameter. The library path lived in
//! the per-user `config.json` (`ns4 library set`), so a project opened on a machine without that
//! setting, or after the library moved, started in the built-in fallback, and the restored edits
//! were overwritten by the program the library worker loaded (`ProgramHold::hold` takes the
//! restored host values as the new baseline). No library bytes are stored: only a path, a size
//! and a short hash (D20, D128).
//!
//! Threads: the host's main thread serializes and restores ([`PersistentField`]), the editor and
//! the library worker read and write the path and status under a mutex, and the audio thread only
//! swaps the two re-arm masks (relaxed atomics, no lock).

use nih_plug::params::persist::PersistentField;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::{Arc, Mutex, MutexGuard};

/// Bytes hashed from the start of the file for [`Identity::head_hash`]. ASSUMED: the first
/// 64 KiB of a `.ns4b` (the first stored entry) differ between libraries and cost no more than
/// a page read on an external volume; a file edited elsewhere keeps the head but changes size.
const HEAD_BYTES: usize = 64 * 1024;

/// Size and header hash of the library file at save time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub size: u64,
    /// FNV-1a 64 of the first [`HEAD_BYTES`] bytes.
    pub head_hash: u64,
}

impl Identity {
    /// Identity of the file at `path`; `None` when it cannot be read.
    pub fn of(path: &Path) -> Option<Self> {
        let mut file = std::fs::File::open(path).ok()?;
        let size = file.metadata().ok()?.len();
        let mut head = Vec::with_capacity(HEAD_BYTES);
        file.by_ref()
            .take(HEAD_BYTES as u64)
            .read_to_end(&mut head)
            .ok()?;
        let head_hash = head.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
        });
        Some(Self { size, head_hash })
    }
}

/// The serialized form (the `library-state` persist key). Every field is optional so an older
/// blob, or one from a newer build, still loads.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryRef {
    #[serde(default)]
    pub version: u32,
    /// Library path as the user's machine had it.
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub identity: Option<Identity>,
    /// Groups of `ProgramHold` the host had moved after the program load (the edits).
    #[serde(default)]
    pub hold_armed: u32,
    /// Touch / Mod 2 flags of `ProgramOverrides` armed at save time.
    #[serde(default)]
    pub overrides_armed: u32,
}

/// Where the library stands, for the editor and `plugin.log`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LibraryStatus {
    /// Nothing set: the built-in sounds play.
    #[default]
    NoLibrary,
    /// The file was found and matches what the project saved (or nothing was saved to compare).
    Ready,
    /// Found, but its size or header differ from what the project was saved with.
    Changed,
    /// The saved path was gone; the same file was found at the user's saved library path.
    Relocated(String),
    /// Not found. The worker tries again every few seconds and on a re-selected path.
    Missing(String),
}

impl LibraryStatus {
    /// The line the editor shows; `None` when nothing needs saying.
    pub fn message(&self) -> Option<String> {
        match self {
            Self::NoLibrary | Self::Ready => None,
            Self::Changed => Some("sound library changed since the project was saved".into()),
            Self::Relocated(from) => Some(format!("sound library found elsewhere (was {from})")),
            Self::Missing(p) => Some(format!(
                "library not found: {p} (the project keeps its settings; mount the volume or open the .ns4b again)"
            )),
        }
    }
}

/// The persisted field of `Stage4Params`.
pub struct LibraryPersist {
    data: Mutex<LibraryRef>,
    status: Mutex<LibraryStatus>,
    /// Live masks of the hold table and the overrides, read when the host saves.
    hold_armed: Arc<AtomicU32>,
    overrides_armed: Arc<AtomicU32>,
    /// Masks restored from the project, applied once after the first program load.
    pending_hold: AtomicU32,
    pending_overrides: AtomicU32,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl LibraryPersist {
    pub fn new(hold_armed: Arc<AtomicU32>, overrides_armed: Arc<AtomicU32>) -> Self {
        Self {
            data: Mutex::default(),
            status: Mutex::default(),
            hold_armed,
            overrides_armed,
            pending_hold: AtomicU32::new(0),
            pending_overrides: AtomicU32::new(0),
        }
    }

    /// The path saved with (or chosen for) this instance.
    pub fn path(&self) -> Option<PathBuf> {
        lock(&self.data).path.as_ref().map(PathBuf::from)
    }

    /// Records `path` (and its identity, when readable) as this instance's library.
    pub fn remember(&self, path: &Path) {
        let mut d = lock(&self.data);
        d.version = 1;
        d.path = Some(path.display().to_string());
        d.identity = Identity::of(path).or(d.identity);
    }

    /// The identity saved with the project.
    pub fn saved_identity(&self) -> Option<Identity> {
        lock(&self.data).identity
    }

    pub fn set_status(&self, s: LibraryStatus) {
        *lock(&self.status) = s;
    }

    pub fn status(&self) -> LibraryStatus {
        lock(&self.status).clone()
    }

    /// Takes the masks restored from the project: `(hold, overrides)`, once. Lock-free.
    pub fn take_rearm(&self) -> (u32, u32) {
        (
            self.pending_hold.swap(0, Relaxed),
            self.pending_overrides.swap(0, Relaxed),
        )
    }
}

impl<'a> PersistentField<'a, LibraryRef> for LibraryPersist {
    fn set(&self, new_value: LibraryRef) {
        self.pending_hold.store(new_value.hold_armed, Relaxed);
        self.pending_overrides
            .store(new_value.overrides_armed, Relaxed);
        *lock(&self.data) = new_value;
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&LibraryRef) -> R,
    {
        let mut v = lock(&self.data).clone();
        v.hold_armed = self.hold_armed.load(Relaxed);
        v.overrides_armed = self.overrides_armed.load(Relaxed);
        f(&v)
    }
}

impl<'a> PersistentField<'a, LibraryRef> for Arc<LibraryPersist> {
    fn set(&self, new_value: LibraryRef) {
        self.as_ref().set(new_value);
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&LibraryRef) -> R,
    {
        self.as_ref().map(f)
    }
}

/// Which file an instance opens and what to tell the user. Pure over its inputs.
///
/// * `env`: `NS4_LIBRARY` / `none` already resolved (`Some(None)` = disabled).
/// * `project`: the path and identity saved with the project.
/// * `user`: the library of the user's `config.json`.
pub fn resolve(
    env: Option<Option<PathBuf>>,
    project: Option<(&Path, Option<Identity>)>,
    user: Option<&Path>,
) -> (Option<PathBuf>, LibraryStatus) {
    if let Some(forced) = env {
        let status = if forced.is_some() {
            LibraryStatus::Ready
        } else {
            LibraryStatus::NoLibrary
        };
        return (forced, status);
    }
    let Some((saved, identity)) = project else {
        return (
            user.map(Path::to_path_buf),
            user.map_or(LibraryStatus::NoLibrary, |_| LibraryStatus::Ready),
        );
    };
    if saved.is_file() {
        let changed = identity.is_some_and(|i| Identity::of(saved) != Some(i));
        let status = if changed {
            LibraryStatus::Changed
        } else {
            LibraryStatus::Ready
        };
        return (Some(saved.to_path_buf()), status);
    }
    // The saved path is gone (another volume name, an unmounted disk). The user's own library
    // stands in only when it is the same file, never another library's A:11.
    if let Some(u) = user.filter(|u| u.is_file()) {
        let same_name = u.file_name() == saved.file_name();
        if same_name || identity.is_some_and(|i| Identity::of(u) == Some(i)) {
            return (
                Some(u.to_path_buf()),
                LibraryStatus::Relocated(saved.display().to_string()),
            );
        }
    }
    (
        Some(saved.to_path_buf()),
        LibraryStatus::Missing(saved.display().to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auto_library::support::two_program_backup;

    fn persist() -> LibraryPersist {
        LibraryPersist::new(Arc::default(), Arc::default())
    }

    fn json(p: &LibraryPersist) -> String {
        p.map(|v| serde_json::to_string(v).unwrap())
    }

    #[test]
    fn the_blob_round_trips_path_identity_and_the_armed_masks() {
        let file = two_program_backup("state-rt");
        let a = persist();
        a.remember(&file);
        a.hold_armed.store(0b101, Relaxed);
        a.overrides_armed.store(0b11, Relaxed);
        let text = json(&a);
        let b = persist();
        b.set(serde_json::from_str(&text).unwrap());
        let _ = std::fs::remove_file(&file);
        assert_eq!(b.path(), Some(file));
        assert_eq!(b.saved_identity(), a.saved_identity());
        assert!(b.saved_identity().is_some());
        assert_eq!(b.take_rearm(), (0b101, 0b11));
        assert_eq!(b.take_rearm(), (0, 0), "applied once");
    }

    #[test]
    fn a_blob_without_the_new_fields_still_loads() {
        let p = persist();
        p.set(serde_json::from_str("{}").unwrap());
        assert_eq!(p.path(), None);
        p.set(serde_json::from_str(r#"{"path":"/a.ns4b","future":1}"#).unwrap());
        assert_eq!(p.path(), Some(PathBuf::from("/a.ns4b")));
        assert_eq!(p.take_rearm(), (0, 0));
    }

    #[test]
    fn identity_tracks_size_and_header_not_the_name() {
        let a = two_program_backup("id-a");
        let b = two_program_backup("id-b");
        let (ia, ib) = (Identity::of(&a), Identity::of(&b));
        std::fs::write(&b, b"x").unwrap();
        let ic = Identity::of(&b);
        let _ = (std::fs::remove_file(&a), std::fs::remove_file(&b));
        assert_eq!(ia, ib, "same bytes, other name");
        assert_ne!(ia, ic);
        assert_eq!(Identity::of(Path::new("/no/such/file")), None);
    }

    #[test]
    fn a_found_file_is_ready_and_a_replaced_one_is_reported_changed() {
        let f = two_program_backup("res-found");
        let id = Identity::of(&f);
        let (p, s) = resolve(None, Some((&f, id)), None);
        assert_eq!((p.as_deref(), s), (Some(f.as_path()), LibraryStatus::Ready));
        std::fs::write(&f, b"another library").unwrap();
        let (_, s) = resolve(None, Some((&f, id)), None);
        let _ = std::fs::remove_file(&f);
        assert_eq!(s, LibraryStatus::Changed);
    }

    #[test]
    fn a_missing_path_keeps_the_path_and_says_so() {
        let gone = Path::new("/Volumes/Nord/Factory.ns4b");
        let (p, s) = resolve(None, Some((gone, None)), None);
        assert_eq!(p.as_deref(), Some(gone), "the worker keeps retrying it");
        assert_eq!(s, LibraryStatus::Missing(gone.display().to_string()));
        assert!(s
            .message()
            .unwrap()
            .starts_with("library not found: /Volumes/Nord/Factory.ns4b"));
    }

    #[test]
    fn the_same_file_at_the_users_library_stands_in_for_a_moved_volume() {
        let f = two_program_backup("res-moved");
        let name = f.file_name().unwrap().to_owned();
        let gone = Path::new("/Volumes/Old").join(&name);
        let (p, s) = resolve(None, Some((&gone, None)), Some(&f));
        assert_eq!(p.as_deref(), Some(f.as_path()));
        assert_eq!(s, LibraryStatus::Relocated(gone.display().to_string()));
        // Another file name with another identity is never substituted.
        let other = Path::new("/Volumes/Old/Other.ns4b");
        let (p, s) = resolve(
            None,
            Some((
                other,
                Some(Identity {
                    size: 1,
                    head_hash: 2,
                }),
            )),
            Some(&f),
        );
        let _ = std::fs::remove_file(&f);
        assert_eq!(p.as_deref(), Some(other));
        assert!(matches!(s, LibraryStatus::Missing(_)));
    }

    #[test]
    fn nothing_saved_follows_the_users_library_and_the_environment_wins() {
        let user = Path::new("/u/lib.ns4b");
        assert_eq!(resolve(None, None, Some(user)).0.as_deref(), Some(user));
        assert_eq!(resolve(None, None, None), (None, LibraryStatus::NoLibrary));
        let env = Some(Some(PathBuf::from("/e.ns4b")));
        let proj = Some((Path::new("/p.ns4b"), None));
        assert_eq!(
            resolve(env, proj, Some(user)).0,
            Some(PathBuf::from("/e.ns4b"))
        );
        assert_eq!(
            resolve(Some(None), proj, Some(user)).0,
            None,
            "NS4_LIBRARY=none"
        );
    }
}
