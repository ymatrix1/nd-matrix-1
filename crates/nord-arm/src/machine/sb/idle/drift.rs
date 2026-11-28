//! Drift guard of idle v2 stage 1.
//!
//! `probe_idle_v2_counted`, `probe_loop_v2` and `run_idle_skip_v2_counted` (in `idle.rs`) mirror
//! by hand three functions of `machine/idle.rs`: `Machine::probe_idle`, `Machine::probe_loop`
//! and `Machine::run_idle_skip`. The work-package row asks for `probe_loop` to be called, never
//! copied, but stage 1 changes the body of that loop (compare only at the start pc, hook and
//! stop checks per page), so the copy is unavoidable without editing `machine/idle.rs`, which
//! this package does not own. Equivalence is proven by the differential tests of this directory
//! for the scenarios they build; this file closes the rest of the gap: it pins the text of the
//! three v1 functions, so a change to any of them (a new stop condition, a new impurity check,
//! another skip rule) fails here and sends the author to the v2 mirror before it can drift.
//!
//! When a pin fails: port the change to the mirror named in its message, extend the differential
//! tests to the new case, then paste the new normalized text over the pin.

/// The source of the oracle (the path is relative to this file).
const V1_SOURCE: &str = include_str!("../../idle.rs");

/// `Machine::probe_idle`, normalized (see [`normalized`]). Mirror: `probe_idle_v2_counted`.
const PROBE_IDLE: &str = r"pub fn probe_idle(&mut self, limit: u64) -> (Option<u64>, Option<Stop>) {
let start = self.cpu.clone();
self.bus.probing = true;
self.bus.impure = false;
let out = self.probe_loop(&start, limit.min(MAX_PERIOD));
self.bus.probing = false;
out
}";

/// `Machine::probe_loop`, normalized. Mirror: `probe_loop_v2`.
const PROBE_LOOP: &str = r"pub(super) fn probe_loop(&mut self, start: &Cpu, n: u64) -> (Option<u64>, Option<Stop>) {
for i in 1..=n {
let pc = self.cpu.r[15];
if Some(pc) == self.stop_pc {
return (None, Some(Stop::StopPc));
}
if self.bus.mem.flags(pc) & HOOK_X != 0 {
return (None, None);
}
self.bus.pc = pc;
let fault = if self.predecode && !self.cpu.thumb && pc & 3 == 0 {
self.step_cached(pc)
} else {
self.cpu.step(&mut self.bus).err()
};
if let Some(f) = fault {
return (None, Some(Stop::Fault(f)));
}
if self.bus.impure {
return (None, None);
}
if same_state(&self.cpu, start) {
return (Some(i), None);
}
}
(None, None)
}";

/// `Machine::run_idle_skip`, normalized. Mirror: `run_idle_skip_v2_counted` (with `wake_at_end`).
const RUN_IDLE_SKIP: &str = r"pub fn run_idle_skip(&mut self, budget: u64, stats: &mut IdleStats) -> Stop {
let end = self.cpu.insns + budget;
let mut slice = MIN_SLICE;
while self.cpu.insns < end {
let remaining = end - self.cpu.insns;
let before = self.cpu.insns;
let (_, stop) = self.run(slice.min(remaining), super::SENTINEL, self.stop_pc);
stats.executed += self.cpu.insns - before;
match stop {
Stop::Budget => {}
Stop::Fault(Fault::Wfi) => {
stats.wfi += 1;
self.wfi += 1;
stats.skipped += end - self.cpu.insns;
self.cpu.insns = end;
return Stop::Budget;
}
other => return other,
}
let remaining = end - self.cpu.insns;
if remaining == 0 {
break;
}
stats.probes += 1;
let before = self.cpu.insns;
let (period, stop) = self.probe_idle(remaining);
stats.executed += self.cpu.insns - before;
stats.probe_executed += self.cpu.insns - before;
if let Some(Stop::Fault(Fault::Wfi)) = stop {
stats.wfi += 1;
self.wfi += 1;
stats.skipped += end - self.cpu.insns;
self.cpu.insns = end;
return Stop::Budget;
}
if let Some(s) = stop {
return s;
}
match period {
Some(l) => {
let k = (end - self.cpu.insns) / l;
self.cpu.insns += k * l;
stats.skips += 1;
stats.skipped += k * l;
slice = MIN_SLICE;
}
None => slice = (slice * 2).min(MAX_SLICE),
}
}
Stop::Budget
}";

/// The text of the item of `src` that starts at `marker` (its signature) and ends at the first
/// closing brace at the indentation of a method (4 spaces), with line endings, indentation,
/// empty lines and `//` comment lines removed. Panics when `marker` or the end is missing.
fn normalized(src: &str, marker: &str) -> String {
    let src = src.replace("\r\n", "\n");
    let start = src
        .find(marker)
        .unwrap_or_else(|| panic!("`{marker}` is gone from machine/idle.rs: re-point this pin"));
    let len = src[start..]
        .find("\n    }\n")
        .unwrap_or_else(|| panic!("`{marker}` has no method-level closing brace"));
    let end = start + len + "\n    }".len();
    src[start..end]
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_v1_functions_that_v2_mirrors_are_unchanged() {
    let pins = [
        ("pub fn probe_idle(", PROBE_IDLE, "probe_idle_v2_counted"),
        ("pub(super) fn probe_loop", PROBE_LOOP, "probe_loop_v2"),
        (
            "pub fn run_idle_skip(",
            RUN_IDLE_SKIP,
            "run_idle_skip_v2_counted",
        ),
    ];
    for (marker, expected, mirror) in pins {
        assert_eq!(
            normalized(V1_SOURCE, marker),
            expected,
            "machine/idle.rs changed `{marker}`, which `{mirror}` in sb/idle.rs mirrors by hand: \
             port the change, extend the differential tests, then update the pin in sb/idle/drift.rs"
        );
    }
}

#[test]
fn the_normalizer_ignores_layout_and_comments_but_sees_every_changed_statement() {
    let src = "impl M {\r\n    /// doc\r\n    pub fn f(&mut self) -> u32 {\r\n        // note\r\n\r\n        \
               let a = 1;\r\n        a\r\n    }\r\n\r\n    fn g() {}\r\n}\r\n";
    let want = "pub fn f(&mut self) -> u32 {\nlet a = 1;\na\n}";
    assert_eq!(normalized(src, "pub fn f("), want);
    // LF endings, other indentation and extra comment lines give the same text
    let relaid = src
        .replace("\r\n", "\n")
        .replace("        let", "      let")
        .replace("a\n    }", "a\n  // x\n    }");
    assert_eq!(normalized(&relaid, "pub fn f("), want);
    // a changed or added statement does not
    let changed = src.replace("let a = 1;", "let a = 2;");
    assert_ne!(normalized(&changed, "pub fn f("), want);
    let added = src.replace("let a = 1;", "let a = 1;\r\n        self.x();");
    assert_ne!(normalized(&added, "pub fn f("), want);
    // the end is the method's own brace, not a nested one
    let nested = src.replace("let a = 1;", "if a {\r\n            b();\r\n        }");
    assert_eq!(
        normalized(&nested, "pub fn f("),
        "pub fn f(&mut self) -> u32 {\nif a {\nb();\n}\na\n}"
    );
}

#[test]
fn the_pins_are_the_real_functions_not_empty_text() {
    let needles: [(&str, &[&str]); 3] = [
        (
            PROBE_IDLE,
            &["probing = true", "probe_loop(", "probing = false"],
        ),
        (
            PROBE_LOOP,
            &[
                "HOOK_X",
                "stop_pc",
                "bus.impure",
                "same_state(",
                "step_cached(",
            ],
        ),
        (
            RUN_IDLE_SKIP,
            &["probe_idle(", "MAX_SLICE", "k * l", "Fault::Wfi"],
        ),
    ];
    for (pin, needles) in needles {
        for needle in needles {
            assert!(pin.contains(needle), "pin lost `{needle}`:\n{pin}");
        }
    }
}
