//! The translation service of one scenario run, kept across the async rounds of a plan.
//!
//! OWNER: W3-FUZZ. Either synchronous (pumped on the calling thread through a scratch core) or one
//! worker thread, as `UopConfig::async_translate` runs it.
use super::scenario::Scenario;
use crate::core::uop::config::UopConfig;
use crate::core::uop::service::worker::scratch::Scratch;
use crate::core::uop::service::Service;
use crate::core::Core;

/// Longest a threaded round waits for the worker. ASSUMED generous (the `service` tests of
/// W1-SVC use the same 30 s): a loaded CI machine still translates a few tiny ranges well inside it.
const DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

/// The translation service of one run, kept across rounds: either synchronous (pumped on the
/// calling thread through a scratch core) or one worker thread (`UopConfig::async_translate`).
pub(super) struct Rig {
    svc: Service,
    /// Built at the first synchronous round (it holds a whole core).
    scratch: Option<Scratch>,
    stop_pc: Option<u32>,
    threaded: bool,
}

impl Rig {
    pub(super) fn new(s: &Scenario) -> Rig {
        let svc = if s.threaded {
            Service::spawn(s.cfg.clone(), s.stop_pc).expect("a worker thread")
        } else {
            Service::new()
        };
        Rig {
            svc,
            scratch: None,
            stop_pc: s.stop_pc,
            threaded: s.threaded,
        }
    }

    /// The async path of [`Step::MaintainAsync`]: revalidate, submit the dropped ranges, wait for
    /// the translation (pump it, or wait for the worker) and install the answers.
    pub(super) fn round(&mut self, got: &mut Core, cfg: &UopConfig) {
        got.uop_maintain_async(&mut self.svc);
        if self.threaded {
            let start = std::time::Instant::now();
            while self.svc.in_flight() > 0 {
                got.uop_collect(&mut self.svc);
                assert!(
                    start.elapsed() < DEADLINE,
                    "the worker did not answer in time"
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        } else {
            let stop = self.stop_pc;
            let scratch = self.scratch.get_or_insert_with(|| Scratch::new(stop));
            self.svc.pump_job(|req| scratch.translate(cfg, &req));
        }
        got.uop_collect(&mut self.svc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_synchronous_and_a_threaded_rig_both_answer_a_round_on_an_unchanged_core() {
        for threaded in [false, true] {
            let mut s = Scenario::new("rig", 1, vec![0x0001; 6], 4);
            s.threaded = threaded;
            let mut got = s.core();
            got.enable_uop(s.cfg.clone(), &s.ranges()).unwrap();
            let mut rig = Rig::new(&s);
            rig.round(&mut got, &s.cfg);
            assert_eq!(got.run_uop(4), None, "threaded={threaded}");
        }
    }
}
