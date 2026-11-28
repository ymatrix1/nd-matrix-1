//! OWNER: W1-VGEN. The "exercised" rule: a differential test that compares two runs must also
//! prove the runs did something, or "no difference" can mean "nothing happened" (every case
//! skipped by a failing condition, every program faulting at its first word, a generator that
//! only draws idle states).
//!
//! A test makes a [`DiffRun`], calls [`DiffRun::note`] with each oracle result it compared, and
//! ends with [`DiffRun::assert_exercised`] and a [`Need`] sized to what the test is about (a
//! store family needs stores, a branch family needs jumps). The counters come from the oracle,
//! never from the tier under test, so a broken tier cannot satisfy them.

use super::oracle::TraceRun;
use crate::cpu::Cpu;

/// What a set of compared cases did on the oracle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DiffRun {
    /// Cases compared.
    pub cases: u64,
    /// Instructions started (a failing condition and a fault each count one).
    pub retired: u64,
    /// Cases in which nothing observable happened: no register, flag, access, jump or fault.
    pub idle: u64,
    /// Bus loads and stores performed.
    pub loads: u64,
    pub stores: u64,
    /// Cases that ended in a fault.
    pub faults: u64,
    /// Cases whose `pc` did not simply advance by 4 per instruction.
    pub jumps: u64,
    /// Cases that changed some of `r0..r14`.
    pub reg_writes: u64,
    /// Cases that changed the flags or `q`.
    pub flag_changes: u64,
}

/// Minimum counters a differential test requires (all zero: nothing; `max_idle_pct` 100: any
/// share of idle cases).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Need {
    pub cases: u64,
    pub retired: u64,
    pub loads: u64,
    pub stores: u64,
    pub faults: u64,
    pub jumps: u64,
    pub reg_writes: u64,
    pub flag_changes: u64,
    /// At most this percentage of the cases may be idle.
    pub max_idle_pct: u64,
}

impl Default for Need {
    fn default() -> Self {
        Self {
            cases: 0,
            retired: 0,
            loads: 0,
            stores: 0,
            faults: 0,
            jumps: 0,
            reg_writes: 0,
            flag_changes: 0,
            max_idle_pct: 100,
        }
    }
}

impl DiffRun {
    /// Count one compared case: the oracle run `t` that started from `before`.
    pub fn note(&mut self, before: &Cpu, t: &TraceRun) {
        let regs = before.r[..15] != t.cpu.r[..15];
        let flags = (before.nzcv(), before.q) != (t.cpu.nzcv(), t.cpu.q);
        let jumped = t.cpu.r[15] != before.r[15].wrapping_add(4 * t.executed as u32);
        let accesses = t.bus.log.len();
        self.cases += 1;
        self.retired += t.executed;
        self.loads += t.bus.loads() as u64;
        self.stores += t.bus.stores() as u64;
        self.faults += u64::from(t.fault.is_some());
        self.jumps += u64::from(jumped);
        self.reg_writes += u64::from(regs);
        self.flag_changes += u64::from(flags);
        self.idle += u64::from(!(regs || flags || jumped || accesses > 0 || t.fault.is_some()));
    }

    /// Add the counters of `o`.
    pub fn merge(&mut self, o: &Self) {
        self.cases += o.cases;
        self.retired += o.retired;
        self.idle += o.idle;
        self.loads += o.loads;
        self.stores += o.stores;
        self.faults += o.faults;
        self.jumps += o.jumps;
        self.reg_writes += o.reg_writes;
        self.flag_changes += o.flag_changes;
    }

    /// `Err` names every counter below `need`.
    pub fn require(&self, need: &Need) -> Result<(), String> {
        let mut short = Vec::new();
        for (name, have, want) in [
            ("cases", self.cases, need.cases),
            ("retired", self.retired, need.retired),
            ("loads", self.loads, need.loads),
            ("stores", self.stores, need.stores),
            ("faults", self.faults, need.faults),
            ("jumps", self.jumps, need.jumps),
            ("reg_writes", self.reg_writes, need.reg_writes),
            ("flag_changes", self.flag_changes, need.flag_changes),
        ] {
            if have < want {
                short.push(format!("{name} {have} < {want}"));
            }
        }
        if self.idle * 100 > need.max_idle_pct * self.cases {
            short.push(format!(
                "idle {} of {} cases > {}%",
                self.idle, self.cases, need.max_idle_pct
            ));
        }
        if short.is_empty() {
            Ok(())
        } else {
            Err(format!("vacuous run ({}): {self:?}", short.join(", ")))
        }
    }

    /// [`DiffRun::require`], panicking with `what` in the message.
    #[track_caller]
    pub fn assert_exercised(&self, what: &str, need: &Need) {
        if let Err(e) = self.require(need) {
            panic!("{what}: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::testkit::{oracle_trace, random_cpu_flags, Rng};

    fn note(word: u32, nzcv: u8, run: &mut DiffRun) {
        let mut cpu = random_cpu_flags(&mut Rng::new(7), 0x1000, nzcv);
        (cpu.r[0], cpu.r[1], cpu.r[2], cpu.r[3]) = (0, 0x2000, 3, 0);
        run.note(&cpu, &oracle_trace(&cpu, &[word], 1, 1));
    }

    #[test]
    fn each_counter_counts_what_it_names_and_only_that() {
        let words = [
            (0xE3A0_0001u32, "reg_writes"), // mov r0,#1
            (0xE581_0000, "stores"),        // str r0,[r1]
            (0xE591_3000, "loads"),         // ldr r3,[r1]
            (0xEA00_0002, "jumps"),         // b +16
            (0xE7F0_00F0, "faults"),        // undefined
            (0xE352_0003, "flag_changes"),  // cmp r2,#3 (r2 == 3: Z set, others vary)
        ];
        for (word, what) in words {
            let mut run = DiffRun::default();
            // flags start at "n z c v" except for the compare, which must change them
            note(
                word,
                if what == "flag_changes" { 0 } else { 0b1111 },
                &mut run,
            );
            let want = DiffRun {
                cases: 1,
                retired: 1,
                loads: u64::from(what == "loads"),
                stores: u64::from(what == "stores"),
                faults: u64::from(what == "faults"),
                jumps: u64::from(what == "jumps"),
                reg_writes: u64::from(what == "reg_writes" || what == "loads"),
                flag_changes: u64::from(what == "flag_changes"),
                idle: 0,
            };
            assert_eq!(run, want, "{what}");
        }
    }

    #[test]
    fn a_skipped_instruction_is_idle_and_a_failed_condition_is_not_a_fault() {
        let mut run = DiffRun::default();
        note(0x0A00_0002, 0b0000, &mut run); // beq with Z clear: not taken
        note(0x0381_0001, 0b0000, &mut run); // orreq r0,r1,#1 with Z clear
        assert_eq!(
            (run.cases, run.retired, run.idle, run.jumps, run.faults),
            (2, 2, 2, 0, 0)
        );
        note(0x0381_0001, 0b0100, &mut run); // Z set: executes
        assert_eq!((run.cases, run.idle, run.reg_writes), (3, 2, 1));
    }

    #[test]
    fn require_names_every_missing_counter_and_the_idle_share() {
        let mut run = DiffRun::default();
        note(0xE3A0_0001, 0, &mut run);
        note(0x0A00_0002, 0, &mut run);
        assert_eq!(run.require(&Need::default()), Ok(()));
        let e = run
            .require(&Need {
                cases: 3,
                stores: 1,
                max_idle_pct: 10,
                ..Need::default()
            })
            .unwrap_err();
        assert!(
            e.contains("cases 2 < 3")
                && e.contains("stores 0 < 1")
                && e.contains("idle 1 of 2 cases > 10%"),
            "{e}"
        );
        assert!(
            !e.contains("loads 0 <"),
            "a met requirement is not listed: {e}"
        );
        let ok = Need {
            cases: 2,
            reg_writes: 1,
            max_idle_pct: 50,
            ..Need::default()
        };
        run.assert_exercised("counts", &ok);
    }

    #[test]
    #[should_panic(expected = "an empty sweep: vacuous run")]
    fn assert_exercised_panics_with_the_name_of_the_test() {
        DiffRun::default().assert_exercised(
            "an empty sweep",
            &Need {
                cases: 1,
                ..Need::default()
            },
        );
    }

    #[test]
    fn merge_adds_every_counter() {
        let (mut a, mut b) = (DiffRun::default(), DiffRun::default());
        note(0xE581_0000, 0, &mut a);
        note(0xE591_3000, 0, &mut b);
        note(0xE3A0_0001, 0, &mut b);
        a.merge(&b);
        assert_eq!(
            (a.cases, a.stores, a.loads, a.reg_writes, a.retired),
            (3, 1, 1, 2, 3)
        );
    }
}
