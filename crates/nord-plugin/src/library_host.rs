//! The plugin's side of the library worker (`auto_library`): starting it, asking for the
//! `library_program` value, and waiting for the first program before the host's first block.

use crate::{auto_library, library_select, NdMatrix1Plugin};

/// How long `initialize` waits for the library program to be decoded (the host may render right
/// after it: a bounce). ASSUMED: above the largest factory piano (218 MB, about 9 s on a desktop)
/// shared by several instances, and below the time after which hosts report a plugin as hung.
/// Past it the program still arrives through `process()` (the async path) and `plugin.log`
/// says so.
pub(crate) const LOAD_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

impl NdMatrix1Plugin {
    /// Starts the library worker for the saved `.ns4b` (once per instance) and asks for the
    /// current `library_program` when it is not the one already asked for (a host
    /// re-initialises after a state load or a sample-rate change); later changes are sent from
    /// `process()`. Returns whether a request went out.
    pub fn start_library(&mut self, path: Option<std::path::PathBuf>) -> bool {
        if self.library.is_none() {
            self.library = auto_library::LibraryLink::start(path, self.load_tx.clone());
        }
        self.request_library_program()
    }

    /// Sends `library_program` to the worker when it differs from the last value sent. No
    /// allocation, no lock. `true` when a request went out; a refused one (queue full, no
    /// worker) is not remembered, so the next block sends it again.
    pub(crate) fn request_library_program(&mut self) -> bool {
        let want = self.params.library_program.value();
        if want == self.last_library_program {
            return false;
        }
        let sent = self.library.as_ref().is_some_and(|link| link.request(want));
        if sent {
            self.last_library_program = want;
        }
        sent
    }

    /// Waits for the worker to finish the program asked for in [`Self::start_library`] and
    /// applies what it queued, so the engine holds the program before the host's first block
    /// (a bounce renders right after `initialize`; SoundCraft builds its render instances
    /// without flagging them offline). Runs on the host's main or control thread, never the
    /// audio thread. Returns whether the worker finished in time.
    pub fn wait_for_library_program(&mut self) -> bool {
        let Some(link) = &self.library else {
            return true;
        };
        let started = std::time::Instant::now();
        let handled = link.wait_handled(self.last_library_program, LOAD_WAIT);
        self.drain_loads();
        auto_library::report(&format!(
            "program {}: {} after {} ms (applied: program={}, pianos={:?}, samples={:?})",
            library_select::label(self.last_library_program),
            if handled { "ready" } else { "still loading" },
            started.elapsed().as_millis(),
            self.loaded.program.is_some(),
            self.loaded
                .pianos
                .iter()
                .map(Option::is_some)
                .collect::<Vec<_>>(),
            self.loaded
                .samples
                .iter()
                .map(Option::is_some)
                .collect::<Vec<_>>()
        ));
        handled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_worker_nothing_is_requested_and_the_wait_is_over_at_once() {
        let mut plugin = NdMatrix1Plugin::default();
        // No library: no worker, so the default A:11 cannot be sent.
        assert!(!plugin.start_library(None));
        assert!(plugin.library.is_none());
        assert!(plugin.wait_for_library_program());
        assert!(LOAD_WAIT.as_secs() == 15);
    }
}
