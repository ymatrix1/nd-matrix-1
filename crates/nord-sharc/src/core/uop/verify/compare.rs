//! The exactness comparator: the first architectural or hidden field in which two cores differ.
//!
//! OWNER: W1-VERIFY. The interpreter is the oracle, so this comparator is deliberately wider than
//! `Core::state_mismatch` (which cannot see the call stack contents, the loop frames, the pending
//! delayed branch, the interrupt controller, the timer, the inactive register banks, the bit FIFO
//! on its own, the float extension of stale registers or the memory). Design 4.6.
//!
//! Not compared (not reachable through a handler, or too costly): the sparse device store and
//! the peripheral models (their traffic is compared through the read and write counters), the
//! diagnostic logs (`watch_hits`, `trace_log`, `skipped_pcs`), the decode and uop caches.
//!
//! Skeleton: complete for every field listed in [`FIELDS`]; W1-VERIFY adds the generated
//! "perturb every field" coverage test over future `Core` fields.
use crate::core::Core;
use std::fmt;

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

/// Every field name [`exact_diff`] can report, in comparison order.
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
    "iop",
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

/// Index and values of the first differing byte of two equal-length images.
fn first_byte_diff(a: &[u8], b: &[u8]) -> Option<ExactDiff> {
    let k = a.iter().zip(b).position(|(x, y)| x != y)?;
    Some(ExactDiff {
        field: "sram",
        a: format!("[{k:#x}] = {:#04x}", a[k]),
        b: format!("[{k:#x}] = {:#04x}", b[k]),
    })
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
    if a.iop != b.iop {
        let k = a.iop.iter().zip(b.iop.iter()).position(|(x, y)| x != y)?;
        return Some(ExactDiff {
            field: "iop",
            a: format!("[{k:#x}] = {:#x}", a.iop[k]),
            b: format!("[{k:#x}] = {:#x}", b.iop[k]),
        });
    }
    if a.sram != b.sram {
        return first_byte_diff(&a.sram, &b.sram);
    }
    None
}

/// A named mutation of one compared field.
#[cfg(test)]
pub(in crate::core) type Perturbation = (&'static str, fn(&mut Core));

/// One perturbation per compared field, for the field-coverage tests of this file and of
/// `snap.rs`: applying entry `k` to a fresh core must make [`exact_diff`] report exactly the
/// named field.
#[cfg(test)]
pub(in crate::core) fn perturbations() -> Vec<Perturbation> {
    use crate::core::interrupts::Serviced;
    use crate::core::types::{LoopFrame, Pending};
    let table: &[Perturbation] = &[
        ("pc", |c| c.pc ^= 1),
        ("steps", |c| c.steps += 1),
        ("r", |c| c.r[3] ^= 1),
        ("s", |c| c.s[3] ^= 1),
        ("i", |c| c.i[3] ^= 1),
        ("m", |c| c.m[3] ^= 1),
        ("l", |c| c.l[3] ^= 1),
        ("b", |c| c.b[3] ^= 1),
        ("f40_x", |c| {
            c.fx.tag[2] = c.r[2];
            c.fx.lo[2] = 1;
        }),
        ("f40_y", |c| {
            c.fx_y.tag[2] = c.s[2];
            c.fx_y.lo[2] = 1;
        }),
        ("mr", |c| c.mr[1] += 1),
        ("mr_y", |c| c.mr_y[0] -= 1),
        ("fifo", |c| c.flags.fifo.bff ^= 1),
        ("fifo_y", |c| c.flags_y.fifo.wrp = 3),
        ("flags", |c| c.flags.az = true),
        ("flags_y", |c| c.flags_y.tf = true),
        ("mode1", |c| c.mode1 ^= 1 << 3),
        ("mode2", |c| c.mode2 ^= 1),
        ("ustat", |c| c.ustat[1] ^= 1),
        ("imask", |c| c.imask ^= 1),
        ("mmask", |c| c.mmask ^= 1),
        ("astat_extra", |c| c.sys.astat_extra[1] ^= 1),
        ("stky", |c| c.sys.stky[0] ^= 1),
        ("sys_status", |c| c.sys.status.push([1, 2, 3])),
        ("pc_overflow", |c| c.sys.pc_overflow = true),
        ("flags_reg", |c| c.sys.flags_reg ^= 1),
        ("flag_pins", |c| c.sys.flag_pins ^= 1),
        ("px", |c| c.sys.px ^= 1),
        ("cb_wrap", |c| c.cb_wrap.set(c.cb_wrap.get() ^ 1)),
        ("call_stack", |c| c.call_stack.push(7)),
        ("loops", |c| c.loops.push(LoopFrame::default())),
        ("pending", |c| {
            c.pending = Some(Pending {
                slots: 3,
                target: 5,
                call: false,
            });
        }),
        ("irptl", |c| c.irq.irptl ^= 1),
        ("lirptl", |c| c.irq.lirptl ^= 1),
        ("irq_idle", |c| c.irq.idle = true),
        ("irq_ivt_base", |c| c.irq.ivt_base ^= 0x100),
        ("irq_window_lo", |c| c.irq.window_lo ^= 1),
        ("irq_window_len", |c| c.irq.window_len ^= 1),
        ("irq_stall", |c| c.irq.stall += 1),
        ("irq_due", |c| c.irq.due += 1),
        ("irq_taken", |c| c.irq.taken += 1),
        ("irq_serviced", |c| {
            c.irq.serviced.push(Serviced {
                vector: 3,
                status_pushed: false,
            });
        }),
        ("irq_scheduled", |c| c.irq.scheduled.push((5, 1))),
        ("irq_wake_at", |c| c.irq.wake_at ^= 1),
        ("timer", |c| c.timer.tcount ^= 1),
        ("banks", |c| c.banks.m[0] ^= 1),
        ("skipped", |c| c.skipped += 1),
        ("device_io", |c| c.device_writes += 1),
        ("sdram_io", |c| c.sdram_reads += 1),
        ("iop", |c| c.iop[5] ^= 1),
        ("sram", |c| c.sram[10] ^= 1),
    ];
    table.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_fresh_cores_are_identical() {
        assert_eq!(exact_diff(&Core::new(&[]), &Core::new(&[])), None);
    }

    #[test]
    fn every_compared_field_is_perturbed_and_detected_under_its_own_name() {
        let table = perturbations();
        let names: Vec<_> = table.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, FIELDS, "the perturbation table follows FIELDS");
        for (name, perturb) in table {
            let (mut a, b) = (Core::new(&[]), Core::new(&[]));
            perturb(&mut a);
            let d = exact_diff(&a, &b).unwrap_or_else(|| panic!("{name} not detected"));
            assert_eq!(d.field, name, "{d}");
            let d = exact_diff(&b, &a).unwrap_or_else(|| panic!("{name} not detected"));
            assert_eq!(d.field, name, "{d}");
        }
    }

    #[test]
    fn a_stale_float_extension_is_not_a_difference_but_a_live_one_is() {
        let (mut a, b) = (Core::new(&[]), Core::new(&[]));
        a.fx.lo[4] = 9; // tag 0 equals r[4] = 0 here, so the extension is live until r[4] moves
        a.r[4] = 5;
        let mut b2 = b;
        b2.r[4] = 5;
        assert_eq!(exact_diff(&a, &b2), None, "stale lo reads as zero in both");
        a.fx.tag[4] = 5;
        assert_eq!(exact_diff(&a, &b2).map(|d| d.field), Some("f40_x"));
    }

    #[test]
    fn the_report_names_the_first_differing_memory_byte() {
        let (mut a, b) = (Core::new(&[]), Core::new(&[]));
        a.sram[0x1234] = 0xAB;
        let d = exact_diff(&a, &b).unwrap();
        assert_eq!((d.field, d.a.as_str()), ("sram", "[0x1234] = 0xab"));
        assert!(d.to_string().starts_with("sram: [0x1234]"));
    }
}
