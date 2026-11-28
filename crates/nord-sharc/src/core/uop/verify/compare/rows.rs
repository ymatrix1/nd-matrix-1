//! The perturbation rows of `coverage.rs`: one per sub-field of every struct `exact_diff` opens,
//! and one per `Core` counter that shares a compared name with another. OWNER: W1-VERIFY.
//! Test-only.
//!
//! A row is `(struct, field, FIELDS name, perturbation)`. Applying the perturbation to one of two
//! equal cores must make `exact_diff` report exactly that name, so a comparison dropped from the
//! comparator (a member of a tuple, the raw byte of the float extension, one value of the access
//! census) fails the row that names it. The rows assume the fixtures of [`prepared`].
use crate::core::float_ops::ExtFile;
use crate::core::interrupts::Serviced;
use crate::core::periph::AccessLog;
use crate::core::types::{LoopFrame, Pending};
use crate::core::uop::verify::gen::{
    fill_astat_extra, fill_fifo, fill_stacks, fill_stale_ext, fill_state, fill_sticky, Rng,
};
use crate::core::Core;

/// `(struct, field, FIELDS name, perturbation)`. A struct that exists once per processing element
/// has a second set of rows under `Name(y)`. The pseudo struct `Core` holds the counters that
/// share one compared name. Perturbations assume [`prepared`].
pub(in crate::core) type Row = (&'static str, &'static str, &'static str, fn(&mut Core));

/// Makes the extension of register `n` live with an effective low byte unlike the one before.
fn bump_ext(fx: &mut ExtFile, r: u32, n: usize) {
    let eff = if fx.tag[n] == r { fx.lo[n] } else { 0 };
    fx.tag[n] = r;
    fx.lo[n] = eff ^ 0x5A;
}

/// Changes the raw byte of a register whose extension reads as zero (stale, or a zero byte), so the
/// effective value stays put and only the hidden state differs.
fn change_dead_lo(fx: &mut ExtFile, r: &[u32; 16]) {
    let n = (0..16)
        .find(|&n| fx.tag[n] != r[n] || fx.lo[n] == 0)
        .expect("a register whose extension reads as zero");
    if fx.tag[n] == r[n] {
        fx.tag[n] = r[n] ^ 1; // a live zero byte: make it stale first
    }
    fx.lo[n] ^= 0x5A;
}

/// Moves the tag of a stale extension with a nonzero byte to another stale value.
fn change_stale_tag(fx: &mut ExtFile, r: &[u32; 16]) {
    let n = (0..16)
        .find(|&n| fx.tag[n] != r[n] && fx.lo[n] != 0)
        .expect("a stale extension with a nonzero byte");
    let t = fx.tag[n].wrapping_add(1);
    fx.tag[n] = if t == r[n] { t.wrapping_add(1) } else { t };
}

/// The census fixture: one address with `reads` reads of `rv` and `writes` writes of `wv`, the
/// writes first, every access at `pc`. One argument moves one value of the census entry.
fn census(reads: u32, writes: u32, rv: u32, wv: u32, pc: u32) -> AccessLog {
    /// ASSUMED: any address the census records; this one is in the peripheral range.
    const ADDR: u32 = 0x3000_0100;
    let mut l = AccessLog::default();
    for _ in 0..writes {
        l.write(ADDR, wv, pc);
    }
    for _ in 0..reads {
        l.read(ADDR, rv, pc);
    }
    l
}

/// The fixture the census rows move one value of: `(reads, writes, rv, wv, pc)`. ASSUMED: arbitrary
/// distinct test values (a pc in the code window); any others do, since every row moves one of them.
const BASE: (u32, u32, u32, u32, u32) = (2, 3, 0x22, 0x11, 0x12_0040);

fn set_census(c: &mut Core, (reads, writes, rv, wv, pc): (u32, u32, u32, u32, u32)) {
    c.periph.as_mut().expect("prepared enables the census").log =
        Some(census(reads, writes, rv, wv, pc));
}

pub(in crate::core) fn nested_rows() -> Vec<Row> {
    let mut v: Vec<Row> = Vec::new();
    macro_rules! rows {
        ($($s:literal . $f:literal => $name:literal : |$c:ident| $e:expr);* $(;)?) => {
            $( v.push(($s, $f, $name, |$c| { $e; })); )*
        };
    }
    rows! {
        "Core"."skipped" => "skipped": |c| c.skipped += 1;
        "Core"."first_skipped" => "skipped": |c| c.first_skipped = Some(c.first_skipped.map_or(0x12_0044, |p| p ^ 4));
        "Core"."device_reads" => "device_io": |c| c.device_reads += 1;
        "Core"."device_writes" => "device_io": |c| c.device_writes += 1;
        "Core"."sdram_reads" => "sdram_io": |c| c.sdram_reads += 1;
        "Core"."sdram_writes" => "sdram_io": |c| c.sdram_writes += 1;
        "IrqState"."irptl" => "irptl": |c| c.irq.irptl ^= 1;
        "IrqState"."lirptl" => "lirptl": |c| c.irq.lirptl ^= 1;
        "IrqState"."serviced" => "irq_serviced": |c| c.irq.serviced.push(Serviced { vector: 2, status_pushed: true });
        "IrqState"."idle" => "irq_idle": |c| c.irq.idle = !c.irq.idle;
        "IrqState"."ivt_base" => "irq_ivt_base": |c| c.irq.ivt_base ^= 0x100;
        "IrqState"."window_lo" => "irq_window_lo": |c| c.irq.window_lo ^= 1;
        "IrqState"."window_len" => "irq_window_len": |c| c.irq.window_len ^= 1;
        "IrqState"."scheduled" => "irq_scheduled": |c| c.irq.scheduled.push((9, 2));
        "IrqState"."stall" => "irq_stall": |c| c.irq.stall ^= 1;
        "IrqState"."due" => "irq_due": |c| c.irq.due ^= 1;
        "IrqState"."wake_at" => "irq_wake_at": |c| c.irq.wake_at ^= 1;
        "IrqState"."taken" => "irq_taken": |c| c.irq.taken ^= 1;
        "Serviced"."vector" => "irq_serviced": |c| c.irq.serviced[0].vector ^= 1;
        "Serviced"."status_pushed" => "irq_serviced": |c| c.irq.serviced[0].status_pushed ^= true;
        "TimerState"."tcount" => "timer": |c| c.timer.tcount ^= 1;
        "TimerState"."tperiod" => "timer": |c| c.timer.tperiod ^= 1;
        "TimerState"."running" => "timer": |c| c.timer.running = !c.timer.running;
        "TimerState"."base_cycle" => "timer": |c| c.timer.base_cycle ^= 1;
        "TimerState"."base_count" => "timer": |c| c.timer.base_count ^= 1;
        "TimerState"."seen" => "timer": |c| c.timer.seen ^= 1;
        "SysState"."stky" => "stky": |c| c.sys.stky[1] ^= 1;
        "SysState"."astat_extra" => "astat_extra": |c| c.sys.astat_extra[0] ^= 1;
        "SysState"."status" => "sys_status": |c| c.sys.status[0][2] ^= 1;
        "SysState"."pc_overflow" => "pc_overflow": |c| c.sys.pc_overflow = !c.sys.pc_overflow;
        "SysState"."flags_reg" => "flags_reg": |c| c.sys.flags_reg ^= 1;
        "SysState"."flag_pins" => "flag_pins": |c| c.sys.flag_pins ^= 1;
        "SysState"."px" => "px": |c| c.sys.px ^= 1 << 40;
        "Banks"."r" => "banks": |c| c.banks.r[7] ^= 1;
        "Banks"."s" => "banks": |c| c.banks.s[7] ^= 1;
        "Banks"."i" => "banks": |c| c.banks.i[7] ^= 1;
        "Banks"."m" => "banks": |c| c.banks.m[7] ^= 1;
        "Banks"."l" => "banks": |c| c.banks.l[7] ^= 1;
        "Banks"."b" => "banks": |c| c.banks.b[7] ^= 1;
        // a live extension: the effective 40-bit value moves
        "ExtFile"."tag" => "f40_x": |c| bump_ext(&mut c.fx, c.r[5], 5);
        "ExtFile"."lo" => "f40_x": |c| bump_ext(&mut c.fx, c.r[6], 6);
        "ExtFile(y)"."tag" => "f40_y": |c| bump_ext(&mut c.fx_y, c.s[5], 5);
        "ExtFile(y)"."lo" => "f40_y": |c| bump_ext(&mut c.fx_y, c.s[6], 6);
        // a stale extension: the effective value stays, the hidden state moves
        "ExtFile"."tag" => "fx_x": |c| change_stale_tag(&mut c.fx, &c.r);
        "ExtFile"."lo" => "fx_x": |c| change_dead_lo(&mut c.fx, &c.r);
        "ExtFile(y)"."tag" => "fx_y": |c| change_stale_tag(&mut c.fx_y, &c.s);
        "ExtFile(y)"."lo" => "fx_y": |c| change_dead_lo(&mut c.fx_y, &c.s);
        "BitFifo"."bff" => "fifo": |c| c.flags.fifo.bff ^= 1 << 63;
        "BitFifo"."wrp" => "fifo": |c| c.flags.fifo.wrp ^= 1;
        "BitFifo(y)"."bff" => "fifo_y": |c| c.flags_y.fifo.bff ^= 1 << 63;
        "BitFifo(y)"."wrp" => "fifo_y": |c| c.flags_y.fifo.wrp ^= 1;
        "Flags"."fifo" => "fifo": |c| c.flags.fifo.wrp ^= 2;
        "Flags(y)"."fifo" => "fifo_y": |c| c.flags_y.fifo.wrp ^= 2;
        "LoopFrame"."start" => "loops": |c| c.loops[0].start ^= 1;
        "LoopFrame"."end" => "loops": |c| c.loops[0].end ^= 1;
        "LoopFrame"."remaining" => "loops": |c| c.loops[0].remaining ^= 1;
        "LoopFrame"."term" => "loops": |c| c.loops[0].term = c.loops[0].term.map_or(Some(3), |_| None);
        "LoopFrame"."check" => "loops": |c| c.loops[0].check ^= 1;
        "LoopFrame"."exit_in" => "loops": |c| c.loops[0].exit_in ^= 1;
        "LoopFrame"."skip_after" => "loops": |c| c.loops[0].skip_after ^= true;
        "LoopFrame"."cs_depth" => "loops": |c| c.loops[0].cs_depth ^= 1;
        "LoopFrame"."detached" => "loops": |c| c.loops[0].detached ^= true;
        "Pending"."slots" => "pending": |c| c.pending.as_mut().unwrap().slots ^= 1;
        "Pending"."target" => "pending": |c| c.pending.as_mut().unwrap().target ^= 1;
        "Pending"."call" => "pending": |c| c.pending.as_mut().unwrap().call ^= true;
        "Periph"."sc57x" => "periph": |c| c.enable_sc57x(0);
        "Periph"."boot" => "periph": |c| c.enable_spi_boot(Vec::new());
        "Periph"."epdma" => "periph": |c| c.enable_epdma();
        "Periph"."epchain" => "periph": |c| c.enable_epchain();
        "Periph"."ringdma" => "periph": |c| c.enable_ringdma();
        "Periph"."log" => "periph_log": |c| c.periph.as_mut().unwrap().log = None;
        "Access"."reads" => "periph_log": |c| set_census(c, (BASE.0 + 1, BASE.1, BASE.2, BASE.3, BASE.4));
        "Access"."writes" => "periph_log": |c| set_census(c, (BASE.0, BASE.1 + 1, BASE.2, BASE.3, BASE.4));
        "Access"."last_read" => "periph_log": |c| set_census(c, (BASE.0, BASE.1, BASE.2 ^ 1, BASE.3, BASE.4));
        "Access"."last_write" => "periph_log": |c| set_census(c, (BASE.0, BASE.1, BASE.2, BASE.3 ^ 1, BASE.4));
        "Access"."last_pc" => "periph_log": |c| set_census(c, (BASE.0, BASE.1, BASE.2, BASE.3, BASE.4 + 1));
    }
    macro_rules! flags {
        ($($f:ident),*) => {
            $(
                v.push(("Flags", stringify!($f), "flags", |c| c.flags.$f = !c.flags.$f));
                v.push(("Flags(y)", stringify!($f), "flags_y", |c| c.flags_y.$f = !c.flags_y.$f));
            )*
        };
    }
    flags!(az, an, av, ac, ai, as_, tf, sz, sv, ss);
    v
}

/// A core whose stacks and queues hold one element each and whose census holds one address, so
/// the rows can change one field of it. Registers 9 of both float files hold a stale extension
/// with a nonzero byte; no model is attached.
pub(super) fn prepared() -> Core {
    let mut c = Core::new(&[]);
    c.loops.push(LoopFrame::default());
    c.pending = Some(Pending {
        slots: 2,
        target: 9,
        call: false,
    });
    c.irq.serviced.push(Serviced {
        vector: 1,
        status_pushed: false,
    });
    c.irq.scheduled.push((1, 1));
    c.sys.status.push([1, 2, 3]);
    c.call_stack.push(4);
    // R9 and S9 are 0, so a tag of anything else is stale
    (c.fx.tag[9], c.fx.lo[9]) = (0xF10A_0001, 0x77);
    (c.fx_y.tag[9], c.fx_y.lo[9]) = (0xF10A_0002, 0x66);
    c.enable_access_log();
    set_census(&mut c, BASE);
    c
}

/// [`prepared`] with every filler of `gen` applied, from one seed.
pub(in crate::core) fn prepared_filled(seed: u64) -> Core {
    let mut c = prepared();
    let mut rng = Rng::new(seed);
    fill_state(&mut c, &mut rng, seed.is_multiple_of(2));
    fill_sticky(&mut c, &mut rng);
    fill_astat_extra(&mut c, &mut rng);
    fill_fifo(&mut c, &mut rng);
    fill_stacks(&mut c, &mut rng, 3, 2);
    fill_stale_ext(&mut c, &mut rng);
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::verify::compare::exact_diff;

    #[test]
    fn the_fixture_has_every_container_filled_and_two_fixtures_are_identical() {
        let c = prepared();
        assert!(c.pending.is_some() && !c.loops.is_empty() && !c.call_stack.is_empty());
        assert!(!c.irq.serviced.is_empty() && !c.irq.scheduled.is_empty());
        assert!(
            c.fx.tag[9] != c.r[9] && c.fx.lo[9] != 0,
            "a stale extension in PEx"
        );
        assert!(c.fx_y.tag[9] != c.s[9] && c.fx_y.lo[9] != 0, "and in PEy");
        let l = crate::core::uop::verify::compare::periph::log(&c).unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(
            (l[0].1.reads, l[0].1.writes),
            (u64::from(BASE.0), u64::from(BASE.1))
        );
        for seed in 0..6 {
            assert_eq!(
                exact_diff(&prepared_filled(seed), &prepared_filled(seed)),
                None
            );
        }
    }

    #[test]
    fn every_census_row_moves_exactly_one_value_of_the_entry() {
        let base = census(BASE.0, BASE.1, BASE.2, BASE.3, BASE.4);
        let one = |l: &AccessLog| l.entries().next().map(|(_, a)| *a).unwrap();
        let b = one(&base);
        for (name, other) in [
            ("reads", census(BASE.0 + 1, BASE.1, BASE.2, BASE.3, BASE.4)),
            ("writes", census(BASE.0, BASE.1 + 1, BASE.2, BASE.3, BASE.4)),
            (
                "last_read",
                census(BASE.0, BASE.1, BASE.2 ^ 1, BASE.3, BASE.4),
            ),
            (
                "last_write",
                census(BASE.0, BASE.1, BASE.2, BASE.3 ^ 1, BASE.4),
            ),
            (
                "last_pc",
                census(BASE.0, BASE.1, BASE.2, BASE.3, BASE.4 + 1),
            ),
        ] {
            let o = one(&other);
            let moved = [
                ("reads", o.reads != b.reads),
                ("writes", o.writes != b.writes),
                ("last_read", o.last_read != b.last_read),
                ("last_write", o.last_write != b.last_write),
                ("last_pc", o.last_pc != b.last_pc),
            ];
            let changed: Vec<_> = moved.iter().filter(|m| m.1).map(|m| m.0).collect();
            assert_eq!(changed, [name], "{name} row moves {changed:?}");
        }
    }

    #[test]
    fn the_extension_helpers_pick_dead_and_stale_registers_and_keep_them_that_way() {
        let c = prepared_filled(4);
        let mut d = prepared_filled(4);
        change_dead_lo(&mut d.fx, &d.r);
        change_stale_tag(&mut d.fx_y, &d.s);
        // the effective 40-bit values did not move: only the hidden state did
        for n in 0..16 {
            assert_eq!(c.f40(n), d.f40(n), "R{n}");
        }
        assert_eq!(exact_diff(&c, &d).map(|x| x.field), Some("fx_x"));
    }
}
