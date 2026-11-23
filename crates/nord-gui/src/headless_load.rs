//! Loads a program by its panel location (`A:14`) from the opened `.ns4b` backup without any
//! click, for repeatable screenshots: `nd-matrix-1 --backup X.ns4b --program A:14 --screenshot ...`.
//!
//! The request goes through the same path as the program dial (`ProgramUi::select_loc`, then the
//! library's load queue), so the panel state is the one a real program change produces.

use crate::audio::AudioHandle;
use crate::library::BackupLibraryState;
use crate::panel::bay_program::store::ProgLoc;
use crate::panel::Stage4PanelState;

/// Location asked for on the command line, `None` when `spec` is a file path or malformed.
pub fn parse_slot(spec: &str) -> Option<ProgLoc> {
    ProgLoc::parse(spec.trim())
}

/// The location still to load.
#[derive(Debug, Default)]
pub struct HeadlessLoad {
    pending: Option<ProgLoc>,
}

impl HeadlessLoad {
    /// Load request from the `--program` value (`None` when it is not a location).
    pub fn new(spec: Option<&str>) -> Self {
        Self {
            pending: spec.and_then(parse_slot),
        }
    }

    /// `true` while a request has not been queued yet.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// One step of the load per frame: the request is taken once the archive layout reached the
    /// Program area. Returns `true` while the load has not finished, so a screenshot must wait.
    pub fn drive(
        &mut self,
        state: &mut Stage4PanelState,
        library: &BackupLibraryState,
        audio: &AudioHandle,
    ) -> bool {
        if let Some(loc) = self.pending {
            if !state.program_ui.library.has_factory_layout() {
                // No archive, or not yet synced: keep waiting only while one is open.
                return library.is_open();
            }
            self.pending = None;
            for cmd in state.program_ui.select_loc(loc) {
                audio.send_command(cmd);
            }
            return true;
        }
        !state.program_ui.requests.is_empty() || library.is_busy()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::LoadRequest;

    #[test]
    fn slot_specs_parse_and_paths_do_not() {
        let loc = parse_slot("A:14").expect("A:14");
        assert_eq!((loc.bank, loc.page, loc.slot), (0, 0, 3));
        assert_eq!(
            parse_slot(" C:87 ").map(|l| l.index()),
            Some(2 * 64 + 56 + 6)
        );
        assert!(parse_slot("x/prog.ns4p").is_none());
        assert!(parse_slot("A:19").is_none(), "button 9 does not exist");
        assert!(parse_slot("I:11").is_none(), "bank I does not exist");
    }

    #[test]
    fn without_a_backup_the_request_is_dropped_not_waited_on() {
        let (audio, _rx) = AudioHandle::new();
        let mut state = Stage4PanelState::default();
        let lib = BackupLibraryState::new();
        let mut load = HeadlessLoad::new(Some("A:14"));
        assert!(!load.drive(&mut state, &lib, &audio));
        assert!(load.is_pending(), "kept for when the archive appears");
        assert!(!HeadlessLoad::new(Some("a.ns4p")).is_pending());
    }

    #[test]
    fn a_synced_layout_queues_the_program_request() {
        let (audio, _rx) = AudioHandle::new();
        let mut state = Stage4PanelState::default();
        state
            .program_ui
            .library
            .set_factory(&[(3, "B3 Chords n Solo".to_string())]);
        // Without a catalog the slot is empty: nothing is queued, the load is over.
        let lib = BackupLibraryState::new();
        let mut load = HeadlessLoad::new(Some("A:14"));
        assert!(load.drive(&mut state, &lib, &audio));
        assert!(!load.is_pending());
        state
            .program_ui
            .requests
            .push(LoadRequest::Program("Program/Bank A/x.ns4p".into()));
        assert!(load.drive(&mut state, &lib, &audio), "queue open");
        state.program_ui.requests.clear();
        assert!(!load.drive(&mut state, &lib, &audio));
    }
}
