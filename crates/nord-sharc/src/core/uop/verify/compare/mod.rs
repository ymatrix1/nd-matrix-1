//! The exactness comparator: the first architectural or hidden field in which two cores differ.
//!
//! OWNER: W1-VERIFY. The interpreter is the oracle, so this comparator is deliberately wider than
//! `Core::state_mismatch` (which cannot see the call stack contents, the loop frames, the pending
//! delayed branch, the interrupt controller, the timer, the inactive register banks, the bit FIFO
//! on its own, the hidden float extension (`fx_x`, `fx_y`: the raw low byte and its tag, which a
//! bank swap or a direct register write leaves behind) or the memory). Design 4.6.
//!
//! Compared memory (`memory.rs`): SRAM with the short-word alias window, the IOP window, the
//! dense SDRAM delay-line window, the sparse device store and the ISA-mode word image.
//!
//! Peripherals (`periph.rs`): which models are attached and the access census when the host
//! enabled it. The internal state of a model is not compared (most models cannot be read), so
//! lockstep on a core with peripherals enables the census on both cores.
//!
//! Not compared, with the reason (the list is the `Skip` rows of `coverage.rs`, which fails when a
//! `Core` field is neither compared nor listed): configuration no instruction writes (`lenient`,
//! `fast_float`, `sharc_plus`, the scripted `preset` reads), diagnostic logs (`watch_*`,
//! `rwatch_*`, `trace_*`, `skipped_pcs`, `site_hits`) and caches and engines (`decode_cache`,
//! `code_gen`, `aot_*`, `uop*`, `jit`). A core under comparison must be configured alike.
//!
//! Tests: `perturb.rs` perturbs every name of `FIELDS`; `coverage.rs` reads the `Core` source,
//! lists every field of `Core` and of the structs `exact_diff` opens, and `rows.rs` perturbs each
//! sub-field of them, including each member of the shared counter names (`skipped`, `device_io`,
//! `sdram_io`).
use crate::core::Core;
use memory::{device_diff, sdram_diff, slice_diff};
use std::fmt;

#[cfg(test)]
mod coverage;
mod memory;
mod periph;
#[cfg(test)]
mod perturb;
#[cfg(test)]
mod rows;

#[cfg(test)]
pub(in crate::core) use perturb::{perturbations, Perturbation};
#[cfg(test)]
pub(in crate::core) use rows::{nested_rows, prepared_filled};

/// The first field in which two cores differ and its two values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExactDiff {
    /// Name of the field, one of [`FIELDS`].
    pub field: &'static str,
    /// Value in the first core (debug text).
    pub a: String,
    /// Value in the second core (debug text).
    pub b: String,
}

impl fmt::Display for ExactDiff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {} vs {}", self.field, self.a, self.b)
    }
}

/// Every field name [`exact_diff`] can report, in comparison order (cheap fields first, the memory
/// images last).
pub const FIELDS: &[&str] = &[
    "pc",
    "steps",
    "r",
    "s",
    "i",
    "m",
    "l",
    "b",
    "f40_x",
    "f40_y",
    "fx_x",
    "fx_y",
    "mr",
    "mr_y",
    "fifo",
    "fifo_y",
    "flags",
    "flags_y",
    "mode1",
    "mode2",
    "ustat",
    "imask",
    "mmask",
    "astat_extra",
    "stky",
    "sys_status",
    "pc_overflow",
    "flags_reg",
    "flag_pins",
    "px",
    "cb_wrap",
    "call_stack",
    "loops",
    "pending",
    "irptl",
    "lirptl",
    "irq_idle",
    "irq_ivt_base",
    "irq_window_lo",
    "irq_window_len",
    "irq_stall",
    "irq_due",
    "irq_taken",
    "irq_serviced",
    "irq_scheduled",
    "irq_wake_at",
    "timer",
    "banks",
    "skipped",
    "device_io",
    "sdram_io",
    "periph",
    "periph_log",
    "iop",
    "isa_code",
    "device",
    "sdram",
    "sram",
];

/// The effective 40-bit float registers: the extension counts only while its tag equals the
/// register (a direct write to the register leaves a stale extension that reads as zero).
fn effective_f40(c: &Core, y: bool) -> [u64; 16] {
    let (r, fx) = if y { (&c.s, &c.fx_y) } else { (&c.r, &c.fx) };
    std::array::from_fn(|n| {
        let lo = if fx.tag[n] == r[n] { fx.lo[n] } else { 0 };
        u64::from(r[n]) << 8 | u64::from(lo)
    })
}

/// The hidden float extension as the file keeps it, per register: the raw low byte and, where
/// that byte is not zero, the tag it belongs to. A stale extension (tag unequal to the register)
/// reads as zero but a bank swap or a write that does not clear it can make it live again, so two
/// cores that agree on the effective value can still diverge later. The tag of a zero byte is
/// left out: live or stale it reads as zero and every writer overwrites or clears both, so no
/// program can tell the two apart.
fn raw_ext(c: &Core, y: bool) -> [(u8, u32); 16] {
    let fx = if y { &c.fx_y } else { &c.fx };
    std::array::from_fn(|n| (fx.lo[n], if fx.lo[n] != 0 { fx.tag[n] } else { 0 }))
}

fn banks_of(c: &Core) -> [[u32; 16]; 6] {
    let b = &c.banks;
    [b.r, b.s, b.i, b.m, b.l, b.b]
}

fn timer_of(c: &Core) -> (u32, u32, bool, u64, u32, u64) {
    let t = &c.timer;
    (
        t.tcount,
        t.tperiod,
        t.running,
        t.base_cycle,
        t.base_count,
        t.seen,
    )
}

/// Compares `a` and `b` field by field and returns the first difference, or `None` when the two
/// cores are architecturally and hidden-state identical.
pub fn exact_diff(a: &Core, b: &Core) -> Option<ExactDiff> {
    macro_rules! same {
        ($name:literal, $x:expr, $y:expr) => {{
            let (x, y) = ($x, $y);
            if x != y {
                return Some(ExactDiff {
                    field: $name,
                    a: format!("{x:?}"),
                    b: format!("{y:?}"),
                });
            }
        }};
    }
    // textual compare for types without `PartialEq` (loop frames, the pending branch)
    macro_rules! same_debug {
        ($name:literal, $x:expr, $y:expr) => {{
            let (x, y) = (format!("{:?}", $x), format!("{:?}", $y));
            if x != y {
                return Some(ExactDiff {
                    field: $name,
                    a: x,
                    b: y,
                });
            }
        }};
    }
    same!("pc", a.pc, b.pc);
    same!("steps", a.steps, b.steps);
    same!("r", a.r, b.r);
    same!("s", a.s, b.s);
    same!("i", a.i, b.i);
    same!("m", a.m, b.m);
    same!("l", a.l, b.l);
    same!("b", a.b, b.b);
    same!("f40_x", effective_f40(a, false), effective_f40(b, false));
    same!("f40_y", effective_f40(a, true), effective_f40(b, true));
    same!("fx_x", raw_ext(a, false), raw_ext(b, false));
    same!("fx_y", raw_ext(a, true), raw_ext(b, true));
    same!("mr", a.mr, b.mr);
    same!("mr_y", a.mr_y, b.mr_y);
    same!("fifo", a.flags.fifo, b.flags.fifo);
    same!("fifo_y", a.flags_y.fifo, b.flags_y.fifo);
    same!("flags", a.flags, b.flags);
    same!("flags_y", a.flags_y, b.flags_y);
    same!("mode1", a.mode1, b.mode1);
    same!("mode2", a.mode2, b.mode2);
    same!("ustat", a.ustat, b.ustat);
    same!("imask", a.imask, b.imask);
    same!("mmask", a.mmask, b.mmask);
    same!("astat_extra", a.sys.astat_extra, b.sys.astat_extra);
    same!("stky", a.sys.stky, b.sys.stky);
    same!("sys_status", &a.sys.status, &b.sys.status);
    same!("pc_overflow", a.sys.pc_overflow, b.sys.pc_overflow);
    same!("flags_reg", a.sys.flags_reg, b.sys.flags_reg);
    same!("flag_pins", a.sys.flag_pins, b.sys.flag_pins);
    same!("px", a.sys.px, b.sys.px);
    same!("cb_wrap", a.cb_wrap.get(), b.cb_wrap.get());
    same!("call_stack", &a.call_stack, &b.call_stack);
    same_debug!("loops", a.loops, b.loops);
    same_debug!("pending", a.pending, b.pending);
    same!("irptl", a.irq.irptl, b.irq.irptl);
    same!("lirptl", a.irq.lirptl, b.irq.lirptl);
    same!("irq_idle", a.irq.idle, b.irq.idle);
    same!("irq_ivt_base", a.irq.ivt_base, b.irq.ivt_base);
    same!("irq_window_lo", a.irq.window_lo, b.irq.window_lo);
    same!("irq_window_len", a.irq.window_len, b.irq.window_len);
    same!("irq_stall", a.irq.stall, b.irq.stall);
    same!("irq_due", a.irq.due, b.irq.due);
    same!("irq_taken", a.irq.taken, b.irq.taken);
    same_debug!("irq_serviced", a.irq.serviced, b.irq.serviced);
    same!("irq_scheduled", &a.irq.scheduled, &b.irq.scheduled);
    same!("irq_wake_at", a.irq.wake_at, b.irq.wake_at);
    same!("timer", timer_of(a), timer_of(b));
    same!("banks", banks_of(a), banks_of(b));
    same!(
        "skipped",
        (a.skipped, a.first_skipped),
        (b.skipped, b.first_skipped)
    );
    same!(
        "device_io",
        (a.device_reads, a.device_writes),
        (b.device_reads, b.device_writes)
    );
    same!(
        "sdram_io",
        (a.sdram_reads, a.sdram_writes),
        (b.sdram_reads, b.sdram_writes)
    );
    same!("periph", periph::models(a), periph::models(b));
    same!("periph_log", periph::log(a), periph::log(b));
    // the memory images, cheapest first and only when everything before them matched
    slice_diff("iop", &a.iop, &b.iop)
        .or_else(|| slice_diff("isa_code", &a.isa_code, &b.isa_code))
        .or_else(|| device_diff(&a.device, &b.device))
        .or_else(|| sdram_diff(&a.sdram, &b.sdram))
        .or_else(|| slice_diff("sram", &a.sram, &b.sram))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_fresh_cores_are_identical() {
        assert_eq!(exact_diff(&Core::new(&[]), &Core::new(&[])), None);
    }

    #[test]
    fn fields_are_listed_in_the_order_exact_diff_reports_them_and_are_unique() {
        let mut names = FIELDS.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), FIELDS.len());
        assert_eq!((FIELDS[0], FIELDS[FIELDS.len() - 1]), ("pc", "sram"));
    }

    /// A core whose register `n` of PEx holds `r` with the extension `(tag, lo)`.
    fn with_ext(n: usize, r: u32, tag: u32, lo: u8) -> Core {
        let mut c = Core::new(&[]);
        c.r[n] = r;
        (c.fx.tag[n], c.fx.lo[n]) = (tag, lo);
        c
    }

    #[test]
    fn a_stale_nonzero_extension_is_a_difference_a_dead_one_is_not() {
        // stale, same byte, same tag: identical
        let (a, b) = (with_ext(4, 5, 9, 0xAB), with_ext(4, 5, 9, 0xAB));
        assert_eq!(exact_diff(&a, &b), None);
        // stale, the byte differs: the effective value is the same, the hidden state is not
        let b = with_ext(4, 5, 9, 0);
        assert_eq!(effective_f40(&a, false), effective_f40(&b, false));
        let d = exact_diff(&a, &b).expect("the raw byte differs");
        assert_eq!(d.field, "fx_x", "{d}");
        // stale, same nonzero byte, another tag: a swap back to either value reads differently
        let b = with_ext(4, 5, 10, 0xAB);
        assert_eq!(exact_diff(&a, &b).map(|d| d.field), Some("fx_x"));
        // a zero byte reads as zero whether the tag matches the register or not
        let (live, stale) = (with_ext(4, 5, 5, 0), with_ext(4, 5, 9, 0));
        assert_eq!(exact_diff(&live, &stale), None);
        // live against stale with the same byte: the effective value differs, reported first
        let live = with_ext(4, 5, 5, 0xAB);
        assert_eq!(exact_diff(&live, &a).map(|d| d.field), Some("f40_x"));
        // the second processing element has its own pair of names
        let (mut a, b) = (Core::new(&[]), Core::new(&[]));
        (a.fx_y.tag[7], a.fx_y.lo[7]) = (9, 3); // stale: S7 is 0
        assert_eq!(exact_diff(&a, &b).map(|d| d.field), Some("fx_y"));
    }

    #[test]
    fn a_handler_that_clears_a_stale_extension_the_interpreter_keeps_is_caught_at_once() {
        // R3 holds the float F with the extension 0xAB (live); the secondary file holds X.
        const F: u32 = 0x3F80_0000;
        // VERIFIED: SRRFL is MODE1 bit 10 and exchanges R0-R7 and S0-S7 with the inactive file
        // (`banks.rs` FILE_SETS); the assertion on `banks.r[3]` below fails if that changes.
        const SRRFL: u32 = 1 << 10;
        let make = || {
            let mut c = Core::new(&[]);
            c.r[3] = F;
            c.banks.r[3] = 0x4000_0000;
            (c.fx.tag[3], c.fx.lo[3]) = (F, 0xAB);
            c
        };
        let (mut interp, mut uop) = (make(), make());
        for c in [&mut interp, &mut uop] {
            c.set_mode1(c.mode1 ^ SRRFL); // R3 now holds X, the extension is stale and still 0xAB
        }
        assert_eq!((interp.r[3], interp.banks.r[3]), (0x4000_0000, F));
        assert_eq!((interp.banks.r[3], interp.fx.lo[3]), (F, 0xAB));
        uop.fx.lo[3] = 0; // the handler cleared it
        assert_eq!(
            effective_f40(&interp, false),
            effective_f40(&uop, false),
            "the effective view that state_mismatch reads is blind to this"
        );
        let d = exact_diff(&interp, &uop).expect("the hidden byte differs");
        assert_eq!(d.field, "fx_x", "{d}");
        // the swap back would have made the difference architectural
        for c in [&mut interp, &mut uop] {
            c.set_mode1(c.mode1 ^ SRRFL);
        }
        assert_eq!(exact_diff(&interp, &uop).map(|d| d.field), Some("f40_x"));
    }
}
