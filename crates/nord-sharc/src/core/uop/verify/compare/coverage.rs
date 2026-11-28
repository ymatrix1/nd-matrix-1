//! Field coverage of `exact_diff`, generated from the source of `Core`. OWNER: W1-VERIFY. Test-only.
//!
//! The comparator is only as good as its field list, and `Core` grows (a hidden state added next
//! quarter would silently escape every differential test). This file reads the declaration of
//! `Core` and of each struct the comparator opens field by field, and demands that every field is
//! either perturbed here and detected under a name of `FIELDS`, or listed as `Skip` with the
//! reason it is not compared. A field added to one of those structs fails
//! `every_core_field_is_compared_or_skipped` (or its nested twin) until its row is added.
use super::{exact_diff, FIELDS};
use crate::core::float_ops::ExtFile;
use crate::core::interrupts::Serviced;
use crate::core::types::{LoopFrame, Pending};
use crate::core::uop::verify::gen::{
    fill_astat_extra, fill_fifo, fill_stacks, fill_state, fill_sticky, Rng,
};
use crate::core::Core;
use std::collections::BTreeSet;

const CORE_SRC: &str = include_str!("../../../mod.rs");
const STRUCTS: &[(&str, &str)] = &[
    ("IrqState", include_str!("../../../interrupts.rs")),
    ("Serviced", include_str!("../../../interrupts.rs")),
    ("TimerState", include_str!("../../../timer.rs")),
    ("SysState", include_str!("../../../sysregs.rs")),
    ("Banks", include_str!("../../../banks.rs")),
    ("ExtFile", include_str!("../../../float_ops.rs")),
    ("Flags", include_str!("../../../types.rs")),
    ("LoopFrame", include_str!("../../../types.rs")),
    ("Pending", include_str!("../../../types.rs")),
    ("BitFifo", include_str!("../../../bitfifo.rs")),
];

/// What `exact_diff` does with a `Core` field.
enum Cover {
    /// Compared under this name of `FIELDS`.
    Field(&'static str),
    /// Compared through the fields of the struct of this name (rows of `nested_rows`).
    Nested(&'static str),
    /// Not compared, and why.
    Skip(&'static str),
}
use Cover::{Field, Nested, Skip};

const CONFIG: &str = "configuration set by the host, no instruction writes it";
const DIAG: &str = "diagnostic log or opt-in counter, not architectural state";
const CACHE: &str = "cache or engine state, the thing under test or derived from the code";

#[rustfmt::skip]
const CORE: &[(&str, Cover)] = &[
    ("pc", Field("pc")), ("r", Field("r")), ("s", Field("s")), ("i", Field("i")),
    ("m", Field("m")), ("l", Field("l")), ("b", Field("b")), ("mode1", Field("mode1")),
    ("mode2", Field("mode2")), ("ustat", Field("ustat")), ("mr", Field("mr")),
    ("lenient", Skip(CONFIG)), ("fast_float", Skip(CONFIG)), ("sharc_plus", Skip(CONFIG)),
    ("skipped", Field("skipped")), ("first_skipped", Field("skipped")),
    ("skipped_pcs", Skip(DIAG)), ("watch", Skip(DIAG)), ("watch_len", Skip(DIAG)),
    ("watch_hits", Skip(DIAG)), ("watch_addrs", Skip(DIAG)), ("watch_limit", Skip(DIAG)),
    ("rwatch", Skip(DIAG)), ("trace", Skip(DIAG)), ("trace_dm", Skip(DIAG)),
    ("trace_log", Skip(DIAG)), ("rwatch_len", Skip(DIAG)), ("rwatch_hits", Skip(DIAG)),
    ("rwatch_on", Skip(DIAG)), ("imask", Field("imask")), ("mmask", Field("mmask")),
    ("flags", Nested("Flags")), ("flags_y", Nested("Flags")), ("mr_y", Field("mr_y")),
    ("fx", Nested("ExtFile")), ("fx_y", Nested("ExtFile")), ("steps", Field("steps")),
    ("site_hits", Skip(DIAG)), ("sram", Field("sram")), ("iop", Field("iop")),
    ("device", Field("device")), ("sdram", Field("sdram")), ("sdram_reads", Field("sdram_io")),
    ("sdram_writes", Field("sdram_io")), ("preset", Skip("scripted device reads, set by the host")),
    ("device_reads", Field("device_io")), ("device_writes", Field("device_io")),
    ("call_stack", Field("call_stack")), ("loops", Nested("LoopFrame")),
    ("pending", Nested("Pending")), ("irq", Nested("IrqState")), ("timer", Nested("TimerState")),
    ("sys", Nested("SysState")), ("banks", Nested("Banks")), ("cb_wrap", Field("cb_wrap")),
    ("isa_code", Field("isa_code")),
    ("periph", Skip("peripheral models have no comparator; their traffic is compared as device_io and shadow mode skips such cores")),
    ("decode_cache", Skip(CACHE)), ("code_gen", Skip(CACHE)), ("sram_code_nw_lo", Skip(CACHE)),
    ("sram_code_nw_hi", Skip(CACHE)), ("aot_hot", Skip(CACHE)), ("aot_lo", Skip(CACHE)),
    ("aot_span", Skip(CACHE)), ("aot_bits", Skip(CACHE)), ("aot_slow", Skip(CACHE)),
    ("uop", Skip(CACHE)), ("uop_run", Skip(CACHE)), ("jit", Skip(CACHE)),
];

/// `(struct, field, FIELDS name, perturbation)`. A struct that exists once per processing element
/// has a second set of rows under `Name(y)`. Perturbations assume [`prepared`].
pub(in crate::core) type Row = (&'static str, &'static str, &'static str, fn(&mut Core));

/// Makes the extension of register `n` live with an effective low byte unlike the one before.
fn bump_ext(fx: &mut ExtFile, r: u32, n: usize) {
    let eff = if fx.tag[n] == r { fx.lo[n] } else { 0 };
    fx.tag[n] = r;
    fx.lo[n] = eff ^ 0x5A;
}

pub(in crate::core) fn nested_rows() -> Vec<Row> {
    let mut v: Vec<Row> = Vec::new();
    macro_rules! rows {
        ($($s:literal . $f:literal => $name:literal : |$c:ident| $e:expr);* $(;)?) => {
            $( v.push(($s, $f, $name, |$c| { $e; })); )*
        };
    }
    rows! {
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
        "ExtFile"."tag" => "f40_x": |c| bump_ext(&mut c.fx, c.r[5], 5);
        "ExtFile"."lo" => "f40_x": |c| bump_ext(&mut c.fx, c.r[6], 6);
        "ExtFile(y)"."tag" => "f40_y": |c| bump_ext(&mut c.fx_y, c.s[5], 5);
        "ExtFile(y)"."lo" => "f40_y": |c| bump_ext(&mut c.fx_y, c.s[6], 6);
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

/// A core whose stacks and queues hold one element each, so the rows can change one field of it.
fn prepared() -> Core {
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
    c
}

/// Field names of `pub struct NAME {` in `src` (rustfmt layout: one field per line, four spaces).
fn struct_fields(src: &str, name: &str) -> Vec<String> {
    let heads = ["pub", "pub(super)", "pub(crate)"].map(|p| format!("{p} struct {name} {{"));
    src.lines()
        .skip_while(|l| !heads.contains(&l.to_string()))
        .skip(1)
        .take_while(|l| *l != "}")
        .filter(|l| l.starts_with("    ") && !l.starts_with("     "))
        .map(str::trim_start)
        .filter(|l| !l.starts_with("//") && !l.starts_with("#["))
        .filter_map(|l| {
            let l = match l.strip_prefix("pub(") {
                Some(rest) => rest.split_once(") ").map_or(rest, |(_, r)| r),
                None => l.strip_prefix("pub ").unwrap_or(l),
            };
            // `name: Type`, not the tail of a type that rustfmt wrapped onto several lines
            let (name, _) = l.split_once(": ")?;
            let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
            (!name.is_empty() && name.chars().all(ident)).then(|| name.to_string())
        })
        .collect()
}

fn base(key: &str) -> &str {
    key.strip_suffix("(y)").unwrap_or(key)
}

fn set<'a>(it: impl IntoIterator<Item = &'a str>) -> BTreeSet<&'a str> {
    it.into_iter().collect()
}

/// `(unclassified, stale)`: the parsed fields no row lists, and the listed rows no field has.
fn classification_gaps<'a>(
    parsed: &'a [String],
    listed: &[&'a str],
) -> (Vec<&'a str>, Vec<&'a str>) {
    let p = set(parsed.iter().map(String::as_str));
    let l = set(listed.iter().copied());
    (
        p.difference(&l).copied().collect(),
        l.difference(&p).copied().collect(),
    )
}

#[test]
fn every_core_field_is_compared_or_skipped() {
    let parsed = struct_fields(CORE_SRC, "Core");
    assert!(
        parsed.len() > 60,
        "the parser found {} fields",
        parsed.len()
    );
    let listed: Vec<&str> = CORE.iter().map(|r| r.0).collect();
    let (unclassified, stale) = classification_gaps(&parsed, &listed);
    assert!(
        unclassified.is_empty(),
        "Core fields neither compared nor skipped: {unclassified:?} (add a CORE row, a \
         perturbation for it in perturb.rs and a comparison in exact_diff, or a Skip reason)"
    );
    assert!(
        stale.is_empty(),
        "CORE rows for fields Core lost: {stale:?}"
    );
}

#[test]
fn a_field_added_to_or_dropped_from_a_struct_is_reported() {
    // the failure the two tests around this one exist to raise
    let src = "pub struct Core {\n    pub pc: u32,\n    /// new\n    pub hidden_new: u8,\n}\n";
    let parsed = struct_fields(src, "Core");
    let (new, stale) = classification_gaps(&parsed, &["pc", "gone"]);
    assert_eq!((new, stale), (vec!["hidden_new"], vec!["gone"]));
    let (new, stale) = classification_gaps(&parsed, &["pc", "hidden_new"]);
    assert!(new.is_empty() && stale.is_empty());
}

#[test]
fn every_field_of_the_opened_structs_has_a_perturbation_row() {
    let rows = nested_rows();
    for (name, src) in STRUCTS {
        let parsed = struct_fields(src, name);
        assert!(!parsed.is_empty(), "no fields parsed for {name}");
        let covered: Vec<&str> = rows
            .iter()
            .filter(|r| base(r.0) == *name)
            .map(|r| r.1)
            .collect();
        let (missing, stale) = classification_gaps(&parsed, &covered);
        assert!(
            missing.is_empty(),
            "{name}: fields without a row: {missing:?}"
        );
        assert!(
            stale.is_empty(),
            "{name}: rows for fields it lost: {stale:?}"
        );
    }
}

#[test]
fn compared_names_exist_and_every_comparator_field_is_fed_by_some_core_field() {
    let rows = nested_rows();
    for (field, cover) in CORE {
        match cover {
            Field(n) => assert!(FIELDS.contains(n), "{field}: {n} is not in FIELDS"),
            Nested(s) => assert!(
                rows.iter().any(|r| base(r.0) == *s),
                "{field}: no rows for {s}"
            ),
            Skip(why) => assert!(why.len() > 10, "{field}: give a reason"),
        }
    }
    for r in &rows {
        assert!(
            FIELDS.contains(&r.2),
            "{}.{}: {} is not in FIELDS",
            r.0,
            r.1,
            r.2
        );
    }
    let fed = set(CORE
        .iter()
        .filter_map(|(_, c)| if let Field(n) = c { Some(*n) } else { None })
        .chain(rows.iter().map(|r| r.2)));
    let orphans: Vec<_> = set(FIELDS.iter().copied())
        .difference(&fed)
        .copied()
        .collect();
    assert!(
        orphans.is_empty(),
        "FIELDS no Core field feeds: {orphans:?}"
    );
}

#[test]
fn each_nested_perturbation_is_detected_under_its_name_on_zero_and_filled_states() {
    let rows = nested_rows();
    assert!(rows.len() > 70, "{} rows", rows.len());
    for seed in 0..4u64 {
        for (s, f, name, perturb) in &rows {
            let make = || {
                if seed == 0 {
                    prepared()
                } else {
                    prepared_filled(seed)
                }
            };
            let (mut a, b) = (make(), make());
            assert_eq!(
                exact_diff(&a, &b),
                None,
                "{s}.{f}: the two starts differ (seed {seed})"
            );
            perturb(&mut a);
            let d = exact_diff(&a, &b).unwrap_or_else(|| panic!("{s}.{f} not seen (seed {seed})"));
            assert_eq!(d.field, *name, "{s}.{f} (seed {seed}): {d}");
            let d = exact_diff(&b, &a).unwrap_or_else(|| panic!("{s}.{f} not seen (seed {seed})"));
            assert_eq!(d.field, *name, "{s}.{f} reversed (seed {seed}): {d}");
        }
    }
}

#[test]
fn the_parser_reads_attributes_prefixes_comments_and_continuation_lines() {
    let src =
        "/// doc\npub struct S {\n    /// text\n    pub a: u32,\n    pub(super) b: [u8; 4],\n    \
               pub(in crate::core) c: Vec<(\n        u32,\n        u8,\n    )>,\n    \
               #[cfg(feature = \"jit\")]\n    d: u8,\n}\npub struct T {\n    z: u8,\n}\n";
    assert_eq!(struct_fields(src, "S"), ["a", "b", "c", "d"]);
    assert_eq!(struct_fields(src, "T"), ["z"]);
    assert!(struct_fields(src, "U").is_empty());
}
