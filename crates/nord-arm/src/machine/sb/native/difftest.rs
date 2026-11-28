//! OWNER: W3-NINT. Tier 3 against tier 2 against tier 0 (skeleton: the fallback proof).
//!
//! Spec (design note 8 and 9, WP W3-NINT): layers L0 to L5 with the backend switched to
//! `NativeBackend`: every uop family on random states against `exec::run_sb`, programs on the
//! three-way comparator of `sb::difftest` with `SbConfig::native`, an exit sweep per trace (one
//! run per exit, each state compared) and the `Machine: Send` assertion. When native code is
//! unavailable these tests assert the fallback instead of skipping: `native: true` with no
//! backend runs exactly like the interpreter.

#![allow(dead_code)]

use super::{available, NativeBackend};
use crate::machine::sb::config::SbConfig;
use crate::machine::sb::run::run_sb_full;
use crate::machine::sb::testgen::{machine_with, random_program};
use crate::machine::sb::testkit::{snapshot, Rng};
use crate::machine::sb::SbState;
use crate::machine::Stop;

/// T3 against T0 on one program: `Ok(false)` when native code is unavailable (the fallback ran),
/// `Ok(true)` when the two agreed, `Err` naming the first difference.
pub(crate) fn native_vs_interpreter(words: &[u32], seed: u64, budget: u64) -> Result<bool, String> {
    let mut want = machine_with(words, seed, false);
    let r0 = want.run(budget, 0, None);
    let mut got = machine_with(words, seed, true);
    let mut sb = SbState {
        cfg: SbConfig {
            native: true,
            ..SbConfig::on()
        },
        backend: NativeBackend::new().map(|b| Box::new(b) as Box<_>),
        ..SbState::default()
    };
    let r1 = run_sb_full(&mut got, &mut sb, budget, 0, None);
    let fault = |s: Stop| {
        if let Stop::Fault(f) = s {
            Some(f)
        } else {
            None
        }
    };
    if r0 != r1 {
        return Err(format!("stop {r1:?} vs {r0:?}"));
    }
    match snapshot(&want, fault(r0.1)).diff(&snapshot(&got, fault(r1.1))) {
        Some(d) => Err(d),
        None => Ok(available()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_native_unavailable_the_fallback_equals_the_interpreter() {
        let mut ran = 0;
        for case in 0..20u64 {
            let words = random_program(&mut Rng::new(case + 100), 64);
            match native_vs_interpreter(&words, case, 200) {
                Ok(native) => {
                    assert_eq!(native, available());
                    ran += 1;
                }
                Err(e) => panic!("program {case}: {e}"),
            }
        }
        assert_eq!(ran, 20);
    }

    #[test]
    fn the_machine_stays_send_with_the_native_feature() {
        fn is_send<T: Send>() {}
        is_send::<crate::machine::Machine>();
        is_send::<SbState>();
    }
}
