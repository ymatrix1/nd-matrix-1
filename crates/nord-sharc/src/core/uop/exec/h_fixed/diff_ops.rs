//! Acceptance tests of the fixed-point ALU, multiplier and multiplier-register handlers against
//! `Core::step` (the rig is `diff.rs`; the shifter and the multifunction forms are `diff_wide.rs`).
//!
//! OWNER: W2-FIXED, tests. Every test lowers real instruction words, runs the registered handler
//! on random full states (SIMD on and off, ALUSAT on and off, random conditions) and compares the
//! whole core with the interpreter's; a case the handler bails on is run again through the handler
//! without the bail. What the sweeps covered is counted per kind and per opcode and asserted, so a
//! sweep that ran nothing fails (design P10).
use super::diff::Case;
use super::diff_sweep::{sweep, Tables, Tally};
use super::fixture::{random_programs, t2a};
use super::kinds::{op8, word, ALU_OPS, XALU_BASE, XMRT_K, XMULS_K, XMUL_K};
use crate::core::float::MODE1_ALUSAT;
use crate::core::uop::lower::fixed::valid::mult_kind;
use crate::core::uop::types::{Exit, UopKind};
use crate::core::uop::verify::gen::Rng;
use crate::core::Core;
use std::collections::BTreeSet;

/// What the multiplier sweeps saw: the opcodes that changed the data and the kinds of result.
#[derive(Default)]
struct Seen {
    ops: BTreeSet<u8>,
    /// results that left the 64 bit window (the 80 bit MR2 part is used)
    spill: usize,
    /// MRF and MRB written
    mrf: usize,
    mrb: usize,
    /// only a data register changed, only MR changed, both
    reg_only: usize,
    mr_only: usize,
    both: usize,
}

fn spilled(m: i128) -> bool {
    !(-(1i128 << 63)..(1i128 << 63)).contains(&m)
}

impl Seen {
    fn note(&mut self, c: &Case) {
        if !c.changed() {
            return;
        }
        self.ops.insert(op8(c.a));
        let (b, a) = (&c.before, &c.after);
        let regs = b.r != a.r || b.s != a.s;
        let mr = b.mr != a.mr || b.mr_y != a.mr_y;
        self.reg_only += usize::from(regs && !mr);
        self.mr_only += usize::from(mr && !regs);
        self.both += usize::from(mr && regs);
        self.mrf += usize::from(b.mr[0] != a.mr[0] || b.mr_y[0] != a.mr_y[0]);
        self.mrb += usize::from(b.mr[1] != a.mr[1] || b.mr_y[1] != a.mr_y[1]);
        for k in 0..2 {
            self.spill += usize::from(spilled(a.mr[k]) && a.mr[k] != b.mr[k]);
            self.spill += usize::from(spilled(a.mr_y[k]) && a.mr_y[k] != b.mr_y[k]);
        }
    }
}

/// The counters of `kind` for an assertion message.
fn row(t: &Tally, kind: u16) -> [usize; 6] {
    t.of(kind)
}

#[test]
fn alu_ops_equal_the_interpreter() {
    let programs = random_programs(0, &ALU_OPS, 10, &mut Rng::new(0xA1));
    let (mut ops, mut sat, mut wrap) = (BTreeSet::new(), 0, 0);
    let t = sweep(&programs, 4, &mut |c| {
        if c.changed() {
            ops.insert(op8(c.a));
        }
        if c.raised && c.ran.0 {
            sat += usize::from(c.after.mode1 & MODE1_ALUSAT != 0);
            wrap += usize::from(c.after.mode1 & MODE1_ALUSAT == 0);
        }
    });
    assert_eq!(t.declined, 0, "every modelled opcode is lowered");
    assert_eq!(
        ops.into_iter().collect::<Vec<_>>(),
        ALU_OPS,
        "every opcode changed data somewhere"
    );
    for op in ALU_OPS {
        let k = XALU_BASE + u16::from(op);
        let [ok, slow, changed, pey, raised, forced] = row(&t, k);
        assert_eq!(ok + slow, 80, "op {op:#x}: 10 programs, 2 modes, 4 seeds");
        assert!(
            ok >= 40 && changed >= 25 && pey >= 10,
            "op {op:#x}: {:?}",
            row(&t, k)
        );
        assert_eq!(
            slow, raised,
            "op {op:#x}: it bails exactly when the interpreter raises"
        );
        assert_eq!(
            forced, slow,
            "op {op:#x}: every bail was re-run without the bail"
        );
    }
    // the exceptions did happen: ADD and SUB overflowed, with ALUSAT on and off
    assert!(
        row(&t, XALU_BASE + 0x01)[4] >= 3 && row(&t, XALU_BASE + 0x02)[4] >= 1,
        "{t:?}"
    );
    assert!(sat >= 8 && wrap >= 8, "saturating {sat}, wrapping {wrap}");
}

/// `IF TRUE R3 = op(R1, R2)` (and `S3 = op(S1, S2)` on PEy) on registers set to `regs`, with
/// ALUSAT on or off, through the registered handler and the one without the bail.
fn exact(t: &Tables, op: u8, simd: bool, regs: (u32, u32), alusat: bool) -> (Case, bool) {
    let setup = move |c: &mut Core| {
        (c.r[1], c.r[2], c.s[1], c.s[2]) = (regs.0, regs.1, regs.0, regs.1);
        c.mode1 = if alusat {
            c.mode1 | MODE1_ALUSAT
        } else {
            c.mode1 & !MODE1_ALUSAT
        };
    };
    t.run(&t2a(31, word(op, 3, 1, 2)), 7, simd, &setup)
        .unwrap_or_else(|e| panic!("{e}"))
        .expect("lowered")
}

#[test]
fn saturation_and_wraparound_give_the_interpreters_values() {
    const MAX: u32 = 0x7FFF_FFFF;
    const MIN: u32 = 0x8000_0000;
    // (op, Rx, Ry, result with ALUSAT, result without) at the edges of the 32 bit range: Rx + Ry,
    // Rx - Ry, -Rx (0x22), Rx + 1 (0x29), Rx - 1 (0x2A), ABS Rx (0x30). VERIFIED: the opcode
    // meanings are the arms of `Core::fixed_alu`; the expected values are 2^32 arithmetic, and
    // the handler must agree with the interpreter on each (the rig compares the whole core).
    let cases: [(u8, u32, u32, u32, u32); 8] = [
        (0x01, MAX, 1, MAX, MIN),
        (0x01, MIN, MIN, MIN, 0),
        (0x02, MIN, 1, MIN, MAX),
        (0x02, MAX, u32::MAX, MAX, MIN),
        (0x22, MIN, 0, MAX, MIN),
        (0x29, MAX, 0, MAX, MIN),
        (0x2A, MIN, 0, MIN, MAX),
        (0x30, MIN, 0, MAX, MIN),
    ];
    let tables = Tables::new();
    for (op, rx, ry, sat, wrapped) in cases {
        for simd in [false, true] {
            for alusat in [false, true] {
                let want = if alusat { sat } else { wrapped };
                let at = format!("op {op:#x} {rx:#x} {ry:#x} simd {simd} alusat {alusat}");
                // the real handler leaves the overflow to the interpreter, untouched (the rig
                // checked that), whose result is the expected one; the handler without the bail
                // was then run on the same state and equals the interpreter (the rig checked
                // that too), so its value is the expected one as well
                let (c, forced) = exact(&tables, op, simd, (rx, ry), alusat);
                assert_eq!((c.exit, c.raised, forced), (Exit::Slow, true, true), "{at}");
                assert_eq!(c.after.r[3], want, "{at}: result");
                assert!(!simd || c.after.s[3] == want, "{at}: PEy result");
                assert!(c.after.flags.av, "{at}: AV");
            }
        }
    }
}

#[test]
fn multiplier_ops_equal_the_interpreter() {
    let all: Vec<u8> = (0..=255).collect();
    let programs = random_programs(1, &all, 2, &mut Rng::new(0xB1));
    let mut seen = Seen::default();
    let t = sweep(&programs, 2, &mut |c| seen.note(c));
    // the product opcodes (0x40..=0xFF) each changed data somewhere
    let products: BTreeSet<u8> = (0x40..=0xFFu8).collect();
    let missing: Vec<_> = products.difference(&seen.ops).collect();
    assert!(
        missing.is_empty(),
        "product opcodes that never changed data: {missing:x?}"
    );
    assert!(
        seen.ops.iter().all(|&o| mult_kind(o).is_some()),
        "only lowered opcodes ran"
    );
    assert!(
        t.declined > 20,
        "the codes that are no operation stay with the interpreter"
    );
    // 192 product opcodes and 26 clear, saturate and round opcodes, 2 programs, 2 modes, 2 seeds
    for (k, lowered) in [(XMUL_K, 192 * 8), (XMULS_K, 26 * 8)] {
        let [ok, slow, changed, pey, raised, forced] = row(&t, k);
        assert_eq!(ok + slow, lowered, "kind {k:#x}");
        assert!(
            changed >= ok / 2 && pey >= ok / 4,
            "kind {k:#x}: {:?}",
            row(&t, k)
        );
        assert_eq!(
            (slow, forced),
            (raised, slow),
            "kind {k:#x}: {:?}",
            row(&t, k)
        );
    }
    assert!(
        row(&t, XMUL_K)[4] >= 3,
        "overflows (MV, MOS, MUS) bail: {:?}",
        row(&t, XMUL_K)
    );
    // MRF and MRB, a result in a register or in MR, and results beyond 64 bits (80 bit MR)
    assert!(
        seen.mrf >= 30 && seen.mrb >= 30,
        "MRF {} MRB {}",
        seen.mrf,
        seen.mrb
    );
    assert!(
        seen.reg_only >= 100 && seen.mr_only >= 100,
        "register only {}, MR only {}, both {}",
        seen.reg_only,
        seen.mr_only,
        seen.both
    );
    assert!(seen.spill >= 20, "spills {}", seen.spill);
}

/// MR values at the windows of the saturate, round and clear operations: 32 and 64 bit
/// boundaries, rounding ties, values just beyond, both signs.
fn mr_edges() -> Vec<i128> {
    let wrap = |v: i128| (v << 48) >> 48;
    let mut v = Vec::new();
    for base in [0i128, 1 << 32, 1 << 47, 1 << 63, 1 << 64, 1 << 78] {
        for delta in [-0x8000_0000i128, -1, 0, 1, 0x8000_0000, 0x8000_0001] {
            v.push(wrap(base + delta));
            v.push(wrap(-(base + delta)));
        }
    }
    v.sort_unstable();
    v.dedup();
    v
}

#[test]
fn clear_saturate_and_round_equal_the_interpreter_at_the_window_edges() {
    let tables = Tables::new();
    let edges = mr_edges();
    assert!(edges.len() >= 40, "{} distinct MR values", edges.len());
    let ops: Vec<u8> = (0..=255u8)
        .filter(|&o| mult_kind(o) == Some(UopKind::XMULS))
        .collect();
    assert_eq!(ops.len(), 26);
    let (mut ran, mut bailed, mut forced) = (0, 0, 0);
    let mut moved = BTreeSet::new();
    for &op in &ops {
        let prog = t2a(31, 1 << 20 | word(op, 3, 1, 2));
        for (i, &m) in edges.iter().enumerate() {
            let other = edges[(i * 7 + 3) % edges.len()];
            let setup = move |c: &mut Core| {
                c.mr = [m, other];
                c.mr_y = [other, m];
            };
            let (c, rerun) = tables
                .run(&prog, 11 + i as u64, i % 2 == 0, &setup)
                .unwrap_or_else(|e| panic!("{e}"))
                .expect("lowered");
            ran += 1;
            bailed += usize::from(c.exit == Exit::Slow);
            forced += usize::from(rerun);
            if c.changed() {
                moved.insert(op);
            }
        }
    }
    assert_eq!(ran, 26 * edges.len(), "every op on every value");
    assert!(
        bailed >= 20 && forced == bailed,
        "bailed {bailed} of {ran}, re-run {forced}"
    );
    assert_eq!(
        moved.len(),
        26,
        "every clear, saturate and round opcode changed MR somewhere"
    );
}

#[test]
fn multiplier_register_transfers_equal_the_interpreter() {
    // `Rn = MRxF/B` and `MRxF/B = Rn`: bit 22, bits 21..17 clear, the operation in bits 19..12
    let mut programs = Vec::new();
    for op in 0..32u32 {
        for (n, low) in [(3, 0), (0, 0xA5), (15, 0x5A)] {
            programs.push(t2a(31, 1 << 22 | op << 12 | n << 8 | low));
        }
    }
    let mut seen = Seen::default();
    let t = sweep(&programs, 3, &mut |c| {
        // the other multifunction words of the sweep are the business of `diff_wide.rs`
        if c.kind == XMRT_K {
            seen.note(c);
        }
    });
    let valid = |op: u32| op & 3 != 3 && op & !0x17 == 0;
    let want: BTreeSet<u8> = (0..32u32).filter(|&o| valid(o)).map(|o| o as u8).collect();
    assert_eq!(want.len(), 12, "the transfers: six reads and six writes");
    assert_eq!(seen.ops, want, "every transfer changed data somewhere");
    let [ok, slow, changed, pey, _, _] = row(&t, XMRT_K);
    assert_eq!((ok, slow), (12 * 3 * 2 * 3, 0), "{:?}", row(&t, XMRT_K));
    assert!(changed >= 100 && pey >= 50, "{:?}", row(&t, XMRT_K));
    assert!(
        seen.reg_only > 0 && seen.mr_only > 0,
        "reads write a register, writes write MR"
    );
    assert!(
        seen.mrf > 0 && seen.mrb > 0,
        "foreground and background registers"
    );
}
