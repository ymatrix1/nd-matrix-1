//! The asynchronous install path: dropped ranges go to the translation service, its answers come
//! back as blocks, and a block is installed only after its hash is compared with the live core.
//!
//! OWNER: W2-DRIVE (the service itself is W1-SVC). The service cannot live inside `Core` (the
//! W0 run state has no slot for it and `Core` is not `Send`), so the caller owns it and drives
//! the three steps, once per pass boundary: [`Core::uop_maintain_async`] revalidates and submits,
//! the worker translates on the words captured at request time, [`Core::uop_collect`] installs
//! what came back. A synchronous service (`Service::new`) is pumped between the two by the caller.
//!
//! Rules of the install (P6, design service/):
//!
//! * the words were copied when the request was made and the live code may have changed since:
//!   every block's `code_hash` is recomputed on the live core (`lower::decode::code_hash`, the
//!   function the translator stored it with) and a block whose hash differs is NOT installed. Its
//!   range goes back to `hot.dropped`, so the next `uop_maintain_async` asks again, once per
//!   round and only because the code really changed;
//! * an empty answer is "this range is not translatable" (the worker answers a failed translation
//!   with an empty `TransDone`): nothing is installed and nothing is queued, so the range stays
//!   with the interpreter and is not retried until a write to it drops a block again. No loop;
//! * a block that starts where a live block with the same code already starts is not installed
//!   again (an answer that arrives after a synchronous `uop_maintain` of the same range);
//! * a malformed answer (a block whose uops are not inside `done.uops`) loses that block only.
use crate::core::uop::hot::UopHot;
use crate::core::uop::invalidate::maintain::take_merged;
use crate::core::uop::lower::decode::code_hash;
use crate::core::uop::service::{Service, TransDone, TransReq};
use crate::core::Core;

impl Core {
    /// Revalidates the dirty pages, sends the ranges of the dropped blocks to `svc` and installs
    /// the answers that are ready. Returns the blocks installed by this call. Same entry
    /// condition as [`Core::uop_maintain`]: a pass boundary after a program change.
    pub fn uop_maintain_async(&mut self, svc: &mut Service) -> usize {
        if !self.uop_run.active || self.uop.is_none() {
            return 0;
        }
        self.uop_run.stats.maintain_calls += 1;
        self.uop_revalidate_now();
        self.uop_submit_dropped(svc);
        self.uop_collect(svc)
    }

    /// Sends the ranges waiting in `hot.dropped` to `svc` and returns how many requests it
    /// accepted. A range the service cannot carry (ISA-mode code) is translated here; a range
    /// the full request ring refused stays queued for the next call.
    pub fn uop_submit_dropped(&mut self, svc: &mut Service) -> usize {
        let Some(mut hot) = self.uop.take() else {
            return 0;
        };
        let cfg = self.uop_run.cfg.clone();
        let mut sent = 0;
        for (lo, hi) in take_merged(&mut hot) {
            match TransReq::capture(self, lo, hi) {
                Some(req) => match svc.submit(req) {
                    Ok(()) => sent += 1,
                    Err(_) => hot.dropped.push((lo, hi)),
                },
                None => {
                    if let Ok((blocks, uops)) =
                        crate::core::uop::lower::translate_range(self, &cfg, lo, hi)
                    {
                        self.uop_run.stats.translations += blocks.len() as u64;
                        hot.install(blocks, uops);
                    }
                }
            }
        }
        self.uop = Some(hot);
        sent
    }

    /// Installs every answer `svc` has ready ([`Core::uop_install_done`]); returns the blocks.
    pub fn uop_collect(&mut self, svc: &mut Service) -> usize {
        let mut installed = 0;
        while let Some(done) = svc.poll() {
            installed += self.uop_install_done(done);
        }
        installed
    }

    /// Installs the blocks of one answer whose code is still the code they were built from (see
    /// the module header). Returns the number installed; 0 while the engine is off.
    pub fn uop_install_done(&mut self, done: TransDone) -> usize {
        let Some(mut hot) = self.uop.take() else {
            return 0;
        };
        let installed = self.uop_install_checked(&mut hot, done);
        self.uop = Some(hot);
        installed
    }

    fn uop_install_checked(&mut self, hot: &mut UopHot, done: TransDone) -> usize {
        let (mut blocks, mut uops) = (Vec::new(), Vec::new());
        for mut b in done.blocks {
            let span = b.first as usize..b.first as usize + b.len as usize;
            let Some(src) = done.uops.get(span).filter(|s| !s.is_empty()) else {
                continue;
            };
            if code_hash(self, b.pc, b.end_pc) != b.code_hash {
                hot.dropped.push((b.pc, b.last_pc + 3));
                continue;
            }
            let twin = hot.pcmap(b.simd).get(b.pc).is_some_and(|(id, idx)| {
                let live = &hot.blocks[id as usize];
                idx == 0
                    && live.len != 0
                    && (live.end_pc, live.code_hash) == (b.end_pc, b.code_hash)
            });
            if twin {
                continue;
            }
            b.first = uops.len() as u32;
            uops.extend_from_slice(src);
            self.uop_run.code_pages.set_range(b.pc, b.last_pc + 3);
            blocks.push(b);
        }
        self.uop_run.stats.translations += blocks.len() as u64;
        hot.install(blocks, uops).0 as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::{load, CODE};
    use crate::core::uop::config::{UopConfig, UopError};
    use crate::core::uop::drive::fixture::nop_core;
    use crate::core::uop::service::worker::Scratch;

    /// The three parcels of `R3 = R1 + R2`.
    const ADD: [u16; 3] = [0x013e, 0x0000, 0x1312];

    /// Writes `parcels` through the short-word alias window.
    fn alias(c: &mut Core, at: u32, parcels: &[u16]) {
        for (k, p) in parcels.iter().enumerate() {
            c.dm_write(at + k as u32, u32::from(*p)).unwrap();
        }
    }

    /// An armed core with 24 NOPs, a synchronous service and its translator.
    fn rig() -> (Core, Service, Scratch) {
        let mut c = nop_core(24);
        c.enable_uop(UopConfig::default(), &[(CODE, CODE + 24)])
            .unwrap();
        (c, Service::new(), Scratch::new(None))
    }

    /// One whole round: revalidate and submit, translate on the calling thread, install.
    fn round(c: &mut Core, svc: &mut Service, s: &mut Scratch) -> usize {
        let cfg = UopConfig::default();
        let mut installed = c.uop_maintain_async(svc);
        svc.pump_job(|req| s.translate(&cfg, &req));
        installed += c.uop_collect(svc);
        installed
    }

    #[test]
    fn a_changed_range_goes_through_the_service_and_the_new_code_runs_like_the_interpreter() {
        let (mut got, mut svc, mut scratch) = rig();
        let mut want = nop_core(24);
        load(&mut want, CODE + 8, &ADD);
        alias(&mut got, CODE + 8, &ADD);
        for c in [&mut want, &mut got] {
            (c.r[1], c.r[2]) = (3, 4);
        }
        assert!(got.uop_needs_maintenance());
        assert!(round(&mut got, &mut svc, &mut scratch) > 0);
        assert_eq!((svc.in_flight(), svc.failed()), (0, 0));
        assert!(!got.uop_needs_maintenance());
        assert_eq!(got.uop_stats().maintain_calls, 1);
        assert_eq!((want.run(22), got.run_uop(22)), (None, None));
        assert_eq!(want.state_mismatch(&got), None);
        assert_eq!(got.r[3], 7);
        assert!(got.uop_stats().steps_uop > 0, "{:?}", got.uop_stats());
    }

    #[test]
    fn a_block_whose_code_changed_after_the_request_is_not_installed_and_is_asked_for_again() {
        let (mut c, mut svc, mut scratch) = rig();
        alias(&mut c, CODE + 8, &ADD);
        c.uop_maintain_async(&mut svc); // submits the words as they are now
        assert_eq!(svc.in_flight(), 1);
        // the code changes again before the answer is installed
        alias(&mut c, CODE + 9, &[0x0001, 0x0001]);
        svc.pump_job(|req| scratch.translate(&UopConfig::default(), &req));
        c.uop_collect(&mut svc);
        let hot = c.uop.as_ref().unwrap();
        assert!(
            hot.pcmap(false).get(CODE + 8).is_none() && hot.pcmap(true).get(CODE + 9).is_none(),
            "nothing built from the old words was installed"
        );
        assert!(!hot.dropped.is_empty(), "the range is queued again");
        assert!(c.uop_needs_maintenance());
        // the next round translates the code as it is now
        assert!(round(&mut c, &mut svc, &mut scratch) > 0);
        assert!(c.uop.as_ref().unwrap().pcmap(false).get(CODE + 8).is_some());
        assert!(!c.uop_needs_maintenance());
    }

    #[test]
    fn an_empty_answer_installs_nothing_and_is_not_retried() {
        let (mut c, mut svc, _) = rig();
        alias(&mut c, CODE + 8, &ADD);
        assert_eq!(c.uop_maintain_async(&mut svc), 0);
        svc.pump_job(|_| Err(UopError::Translate("not translatable".into())));
        assert_eq!((svc.failed(), c.uop_collect(&mut svc)), (1, 0));
        assert!(
            !c.uop_needs_maintenance(),
            "nothing is queued: no loop of retries"
        );
        assert_eq!(c.uop_maintain_async(&mut svc), 0);
        assert_eq!(svc.in_flight(), 0, "and nothing is submitted again");
        // the range runs through the interpreter
        let mut want = nop_core(24);
        load(&mut want, CODE + 8, &ADD);
        assert_eq!((want.run(10), c.run_uop(10)), (None, None));
        assert_eq!(want.state_mismatch(&c), None);
    }

    #[test]
    fn a_full_request_ring_keeps_the_ranges_it_refused() {
        // 20 separate two-NOP ranges, all rewritten; a synchronous service that is never pumped
        let ranges: Vec<(u32, u32)> = (0..20).map(|k| (CODE + 8 * k, CODE + 8 * k + 2)).collect();
        let mut c = nop_core(200);
        c.enable_uop(UopConfig::default(), &ranges).unwrap();
        for (lo, _) in &ranges {
            alias(&mut c, *lo, &[0xc8be]);
        }
        let mut svc = Service::new();
        c.uop_maintain_async(&mut svc);
        assert_eq!(
            svc.in_flight(),
            crate::core::uop::service::worker::CAP as u64
        );
        let kept = c.uop.as_ref().unwrap().dropped.len();
        assert_eq!(kept, 20 - crate::core::uop::service::worker::CAP);
        assert!(c.uop_needs_maintenance());
    }

    #[test]
    fn an_answer_for_a_range_that_a_synchronous_maintain_already_translated_is_a_twin() {
        let (mut c, mut svc, mut scratch) = rig();
        alias(&mut c, CODE + 8, &ADD);
        c.uop_maintain_async(&mut svc);
        svc.pump_job(|req| scratch.translate(&UopConfig::default(), &req));
        // before the answer is installed a synchronous maintain translates the same range: put
        // the range back as the dropped list of an earlier revalidation would hold it
        let before = c.uop.as_ref().unwrap().blocks.len();
        c.uop.as_mut().unwrap().dropped.push((CODE, CODE + 26));
        assert!(c.uop_maintain() > 0);
        let mid = c.uop.as_ref().unwrap().blocks.len();
        assert!(mid > before);
        assert_eq!(
            c.uop_collect(&mut svc),
            0,
            "the same blocks are already there"
        );
        assert_eq!(c.uop.as_ref().unwrap().blocks.len(), mid);
    }

    #[test]
    fn a_malformed_answer_loses_the_bad_block_only_and_a_disabled_engine_installs_nothing() {
        let (mut c, mut svc, mut scratch) = rig();
        alias(&mut c, CODE + 8, &ADD);
        c.uop_maintain_async(&mut svc);
        svc.pump_job(|req| scratch.translate(&UopConfig::default(), &req));
        let mut done = svc.poll().expect("one answer");
        assert!(!done.blocks.is_empty());
        let good = done.blocks.len();
        let mut bad = done.blocks[0].clone();
        bad.first = done.uops.len() as u32 + 5; // outside the uop vector
        done.blocks.push(bad);
        assert_eq!(c.uop_install_done(done.clone()), good);
        c.disable_uop();
        assert_eq!(c.uop_install_done(done), 0);
        assert_eq!(c.uop_maintain_async(&mut svc), 0);
        assert_eq!(c.uop_submit_dropped(&mut svc), 0);
    }
}
