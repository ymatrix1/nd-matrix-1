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
    /// Blocks the rounds installed so far (the counts `uop_maintain_async` and `uop_collect`
    /// return). A dropped answer falls back to the interpreter and the state stays exact, so
    /// only this count proves the async path installed anything.
    installed: u64,
    /// Fast steps (`steps_uop - steps_interp_uop`) the engine had run when a round first
    /// installed a block.
    mark: Option<u64>,
}

/// Steps the engine ran through fast uops, from the counters of `core`.
fn fast_steps(core: &Core) -> u64 {
    let st = core.uop_stats();
    st.steps_uop.saturating_sub(st.steps_interp_uop)
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
            installed: 0,
            mark: None,
        }
    }

    /// Blocks installed by every round so far.
    pub(super) fn installed(&self) -> u64 {
        self.installed
    }

    /// Fast steps `core` ran after the first round that installed a block (0 before any did).
    pub(super) fn fast_after_install(&self, core: &Core) -> u64 {
        self.mark.map_or(0, |m| fast_steps(core).saturating_sub(m))
    }

    /// The async path of [`Step::MaintainAsync`]: revalidate, submit the dropped ranges, wait for
    /// the translation (pump it, or wait for the worker) and install the answers. Returns the
    /// blocks this round installed.
    pub(super) fn round(&mut self, got: &mut Core, cfg: &UopConfig) -> usize {
        let mut n = got.uop_maintain_async(&mut self.svc);
        if self.threaded {
            let start = std::time::Instant::now();
            while self.svc.in_flight() > 0 {
                n += got.uop_collect(&mut self.svc);
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
        n += got.uop_collect(&mut self.svc);
        if n > 0 {
            self.mark.get_or_insert_with(|| fast_steps(got));
        }
        self.installed += n as u64;
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::verify::gen::CODE_PC;

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

    #[test]
    fn a_round_after_a_poke_installs_a_block_and_counts_it() {
        for threaded in [false, true] {
            // `R3 = R1 + R2` six times: a fast family, so the block is translatable
            let mut s = Scenario::new("rig", 1, [0x013e, 0x0000, 0x1312].repeat(6), 4);
            s.threaded = threaded;
            let mut got = s.core();
            got.enable_uop(s.cfg.clone(), &s.ranges()).unwrap();
            let mut rig = Rig::new(&s);
            assert_eq!(rig.round(&mut got, &s.cfg), 0, "nothing was dropped yet");
            assert_eq!((rig.installed(), rig.fast_after_install(&got)), (0, 0));
            // the first two instructions (six parcels, three words) become six NOPs
            for w in 0..3 {
                got.dm_write(CODE_PC / 2 + w, 0x0001_0001).unwrap();
            }
            let n = rig.round(&mut got, &s.cfg);
            assert!(
                n >= 1,
                "threaded={threaded}: the dropped range was not installed"
            );
            assert_eq!(rig.installed(), n as u64);
            assert!(rig.mark.is_some(), "the install was not marked");
        }
    }
}
