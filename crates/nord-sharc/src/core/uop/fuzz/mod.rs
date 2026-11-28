//! The long differential fuzz of the uop engine against the interpreter (work package W3-FUZZ).
//!
//! OWNER: W3-FUZZ. Test-only: the whole directory is `#[cfg(test)]` and nothing here is in the
//! library. The interpreter (`Core::step`, through `Core::run`) is the only oracle; every case
//! runs the same program on two identically prepared cores and compares them with `exact_diff`
//! after every slice. Design 6.2, 6.6, 6.8 and 6.9 are covered here:
//!
//! | File | What it fuzzes |
//! |---|---|
//! | `sieve` | the instruction pools per family, found by asking the lowering (census plus random) |
//! | `state` | the start state: hidden state, DAG with live circular buffers, MR edges, IVT |
//! | `shapes` | program shapes: straight, nested loops, delayed branches, calls, system loads |
//! | `scenario`, `driver` | one case end to end, the loop that collects every divergence |
//! | `ran` | the evidence of one run (counters, installs, interrupts taken) |
//! | `rig` | the translation service of a run (synchronous or a worker thread) |
//! | `straight` | random straight programs per family, PEy on and off, block limits |
//! | `control` | jumps, calls, DO and DO UNTIL (nested), delayed branches and their slots |
//! | `events` | interrupts at random cycles, in loops, idle wake-up, exceptions, IMASK/MODE1 writes |
//! | `smc`, `service` | stores into code, host pokes and overlays, async translation rounds |
//! | `checked` | pass loops, `UopChecked`, shadow and translation validation on real families |
//! | `splits`, `mutants` | the exit sweep on any scenario; every `Mutant` class caught |
//! | `findings` | what the fuzz found, pinned (gated by `NS4_FUZZ_STRICT`), the census gate |
//!
//! Knobs (`knobs.rs`): `NS4_FUZZ_CASES` (cases per test, the default is the CI bound),
//! `NS4_FUZZ_SEED` (seed offset), `NS4_FUZZ_STRICT` (a recorded finding fails its test).
//! Every case asserts that it exercised a fast uop (`DiffRun::exercised`, design P10, and a block
//! was entered): a case in which the lowering declined everything is identical to the
//! interpreter by construction and proves nothing.
mod checked;
mod control;
mod driver;
mod events;
mod findings;
mod knobs;
mod mutants;
mod ran;
mod rig;
mod scenario;
mod service;
mod shapes;
mod sieve;
mod smc;
mod splits;
mod state;
mod straight;

#[cfg(test)]
mod tests {
    #[test]
    fn the_fuzz_directory_is_wired_into_the_uop_tree() {
        assert!(module_path!().ends_with("uop::fuzz::tests"));
    }
}
