//! Native OS file-picker helpers for `.ns4b` backups, sound files, and Nord OS updates.
//!
//! The dialog runs on a short-lived worker thread ([`PendingPick`]) and the GUI polls for the
//! answer. A DAW plugin editor runs on the host's UI thread, so a modal dialog opened there would
//! freeze the host window (and nest a second message loop inside the host's); the worker thread
//! keeps both the host and the egui frame loop running while the dialog is open.

use crossbeam_channel::{unbounded as channel, Receiver, TryRecvError};
use std::path::PathBuf;

/// Allowed extensions for the Backup & Sound Library file picker.
pub const BACKUP_AND_SOUND_EXTS: &[&str] = &["ns4b", "ns4p", "npno", "nsmp4"];

/// Allowed extensions for the Nord Stage 4 OS Update file picker.
pub const OS_UPDATE_EXTS: &[&str] = &["zip", "exe", "dmg", "bin"];

/// Which dialog to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickKind {
    /// A `.ns4b` backup, `.ns4p` program, `.npno` piano, or `.nsmp4` sample file.
    BackupOrSound,
    /// A Nord Stage 4 OS update (`.zip`, `.exe`, `.dmg`, `.bin`).
    OsUpdate,
}

/// Answer of a pick that may still be open.
#[derive(Debug, PartialEq, Eq)]
pub enum PickPoll {
    /// The dialog is still open.
    Pending,
    /// The dialog closed: the chosen file, or `None` when cancelled (or the worker died).
    Done(Option<PathBuf>),
}

/// A file dialog running on a worker thread.
pub struct PendingPick {
    rx: Receiver<Option<PathBuf>>,
}

impl PendingPick {
    /// Runs `dialog` on a new thread. `None` when the thread cannot be spawned.
    pub fn spawn_with(dialog: impl FnOnce() -> Option<PathBuf> + Send + 'static) -> Option<Self> {
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("ns4-file-dialog".into())
            .spawn(move || {
                let _ = tx.send(dialog());
            })
            .ok()?;
        Some(Self { rx })
    }

    /// Opens the dialog of `kind` on a worker thread; `None` when this build has no native
    /// dialogs (no `picker` feature) or the thread cannot be spawned.
    pub fn open(kind: PickKind) -> Option<Self> {
        if !cfg!(feature = "picker") {
            return None;
        }
        Self::spawn_with(move || pick_blocking(kind))
    }

    /// Non-blocking: the answer if the dialog has closed.
    pub fn poll(&self) -> PickPoll {
        match self.rx.try_recv() {
            Ok(path) => PickPoll::Done(path),
            Err(TryRecvError::Empty) => PickPoll::Pending,
            Err(TryRecvError::Disconnected) => PickPoll::Done(None),
        }
    }
}

/// Polls the dialog held in `slot`, clearing the slot once it has closed. Returns the chosen
/// file when it closed with one, and keeps the GUI repainting while it is open.
pub fn poll_slot(slot: &mut Option<PendingPick>, ctx: &egui::Context) -> Option<PathBuf> {
    let pick = slot.as_ref()?;
    match pick.poll() {
        PickPoll::Pending => {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            None
        }
        PickPoll::Done(path) => {
            *slot = None;
            path
        }
    }
}

/// Starts the dialog of `kind` into `slot` unless one is already open.
pub fn start_slot(slot: &mut Option<PendingPick>, kind: PickKind) {
    if slot.is_none() {
        *slot = PendingPick::open(kind);
    }
}

/// Filter label and extensions the dialog of `kind` offers (descriptive labels, no brand name).
pub fn filter_for(kind: PickKind) -> (&'static str, &'static [&'static str]) {
    match kind {
        PickKind::BackupOrSound => (
            crate::branding::BACKUP_SOUND_FILTER_LABEL,
            BACKUP_AND_SOUND_EXTS,
        ),
        PickKind::OsUpdate => ("Nord OS Update", OS_UPDATE_EXTS),
    }
}

/// Shows the dialog on the calling thread and blocks until it closes.
#[cfg(feature = "picker")]
fn pick_blocking(kind: PickKind) -> Option<PathBuf> {
    match kind {
        PickKind::BackupOrSound => rfd::FileDialog::new()
            .set_title("Open Nord Stage 4 Backup or Sound File")
            .add_filter(filter_for(kind).0, filter_for(kind).1)
            .add_filter("All Files", &["*"])
            .pick_file(),
        PickKind::OsUpdate => rfd::FileDialog::new()
            .set_title("Select Nord Stage 4 OS Update")
            .add_filter("Nord OS Update", OS_UPDATE_EXTS)
            .add_filter("All Files", &["*"])
            .pick_file(),
    }
}

/// Without the `picker` feature there is no dialog.
#[cfg(not(feature = "picker"))]
fn pick_blocking(_kind: PickKind) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn filter_extensions_cover_all_supported_nord_formats() {
        assert!(BACKUP_AND_SOUND_EXTS.contains(&"ns4b"));
        assert!(BACKUP_AND_SOUND_EXTS.contains(&"ns4p"));
        assert!(BACKUP_AND_SOUND_EXTS.contains(&"npno"));
        assert!(BACKUP_AND_SOUND_EXTS.contains(&"nsmp4"));
        assert!(OS_UPDATE_EXTS.contains(&"zip"));
        assert!(OS_UPDATE_EXTS.contains(&"exe"));
        assert!(OS_UPDATE_EXTS.contains(&"dmg"));
    }

    #[test]
    fn filter_labels_are_descriptive_not_brand_like() {
        let (label, exts) = filter_for(PickKind::BackupOrSound);
        assert_eq!(label, "Backup and sound files for Nord Stage 4");
        assert_ne!(label, "Nord Stage 4 Files");
        assert!(exts.contains(&"ns4b"));
        let (label, exts) = filter_for(PickKind::OsUpdate);
        assert_eq!(label, "Nord OS Update");
        assert!(exts.contains(&"zip"));
    }

    fn wait(pick: &PendingPick) -> PickPoll {
        let start = Instant::now();
        loop {
            match pick.poll() {
                PickPoll::Pending if start.elapsed() < Duration::from_secs(5) => {
                    std::thread::sleep(Duration::from_millis(2));
                }
                other => return other,
            }
        }
    }

    #[test]
    fn a_dialog_answer_arrives_without_blocking_the_caller() {
        let (release_tx, release_rx) = channel::<()>();
        let pick = PendingPick::spawn_with(move || {
            let _ = release_rx.recv();
            Some(PathBuf::from("x.ns4b"))
        })
        .expect("thread");
        // The dialog is open: polling returns at once and reports it pending.
        assert_eq!(pick.poll(), PickPoll::Pending);
        release_tx.send(()).unwrap();
        assert_eq!(wait(&pick), PickPoll::Done(Some(PathBuf::from("x.ns4b"))));
    }

    #[test]
    fn a_cancelled_or_dead_dialog_reports_none() {
        let cancelled = PendingPick::spawn_with(|| None).unwrap();
        assert_eq!(wait(&cancelled), PickPoll::Done(None));
        let dead = PendingPick::spawn_with(|| panic!("dialog crashed")).unwrap();
        assert_eq!(wait(&dead), PickPoll::Done(None));
    }

    #[test]
    fn poll_slot_hands_over_the_file_once_and_empties_the_slot() {
        let ctx = egui::Context::default();
        let mut slot = PendingPick::spawn_with(|| Some(PathBuf::from("a.ns4p")));
        let start = Instant::now();
        let mut got = None;
        while got.is_none() && start.elapsed() < Duration::from_secs(5) {
            got = poll_slot(&mut slot, &ctx);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(got, Some(PathBuf::from("a.ns4p")));
        assert!(slot.is_none());
        assert_eq!(poll_slot(&mut slot, &ctx), None);
    }

    #[test]
    fn start_slot_keeps_a_dialog_that_is_already_open() {
        let (_hold, release_rx) = channel::<()>();
        let mut slot = PendingPick::spawn_with(move || {
            let _ = release_rx.recv();
            None
        });
        start_slot(&mut slot, PickKind::BackupOrSound);
        // Still the first dialog: it is pending, not replaced by a fresh (picker-less) one.
        assert!(slot.as_ref().is_some_and(|p| p.poll() == PickPoll::Pending));
    }

    #[cfg(not(feature = "picker"))]
    #[test]
    fn a_build_without_the_picker_feature_opens_no_dialog() {
        assert!(PendingPick::open(PickKind::BackupOrSound).is_none());
    }
}
