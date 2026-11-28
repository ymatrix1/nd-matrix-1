//! Keeps the host project in step with the editor's Library tab (D262, issue #669): the
//! `.ns4b` the user opened and the program they loaded become the project's saved library path
//! and `library_program`, and a library that cannot be found is said so in the tab.

use crate::params::Stage4Params;
use nih_plug::prelude::*;
use nord_gui::library::BackupLibraryState;

/// `library_program` value of flat location `slot` (0 is Off, so the location plus one).
pub(super) fn program_value(slot: u16) -> i32 {
    i32::from(slot) + 1
}

/// Runs once per editor frame, on the GUI thread.
pub(super) fn sync(library: &mut BackupLibraryState, params: &Stage4Params, setter: &ParamSetter) {
    if let Some(value) = record(library, params) {
        setter.begin_set_parameter(&params.library_program);
        setter.set_parameter(&params.library_program, value);
        setter.end_set_parameter(&params.library_program);
    }
}

/// The state side of [`sync`]: returns the `library_program` value to set, if it changed.
pub(super) fn record(library: &mut BackupLibraryState, params: &Stage4Params) -> Option<i32> {
    let state = &params.library_state;
    if let Some(path) = library.backup_path.as_deref() {
        if state.path().as_deref() != Some(path) {
            state.remember(path);
            state.set_status(crate::library_state::LibraryStatus::Ready);
        }
    }
    if library.error_message.is_none() && !library.status_message.starts_with("Program") {
        library.error_message = state.status().message();
    }
    let value = program_value(library.take_loaded_slot()?);
    (params.library_program.value() != value).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auto_library::support::two_program_backup;
    use crate::library_state::LibraryStatus;

    #[test]
    fn the_opened_backup_becomes_the_projects_library() {
        let file = two_program_backup("sync-open");
        let params = Stage4Params::default();
        let mut library = BackupLibraryState::new();
        library.open_backup_path(&file).expect("opens");
        assert_eq!(record(&mut library, &params), None);
        let _ = std::fs::remove_file(&file);
        assert_eq!(params.library_state.path(), Some(file));
    }

    #[test]
    fn a_missing_library_is_shown_in_the_tab_and_a_location_is_a_one_based_value() {
        let params = Stage4Params::default();
        params
            .library_state
            .set_status(LibraryStatus::Missing("/Volumes/X/f.ns4b".into()));
        let mut library = BackupLibraryState::new();
        library.status_message.clear();
        record(&mut library, &params);
        let msg = library.error_message.clone().expect("shown");
        assert!(msg.starts_with("library not found: /Volumes/X/f.ns4b"));
        assert_eq!(program_value(0), 1, "A:11");
        assert_eq!(program_value(511), 512, "H:88");
    }
}
