//! OWNER: W1-VGEN. Program images, the memory map of the structured generator and the machine a
//! program runs on.
//!
//! [`structured_program`] is the L1 generator of design note 8: a body of constructs from
//! `blocks.rs` (loops, `bl`/`bx lr` pairs, conditional runs, literal loads, stores into the own
//! page, the next word and the literal pool, device accesses, Thumb entry, status moves) placed in
//! one memory map, with callees, a tail and a spin word, so the program always has somewhere to
//! return to and ends by spinning. [`Program::flat`] is the scaffold's flat image.
//!
//! Memory map (offsets from [`CODE`]; one page is 0x1000):
//!
//! | Range | Content |
//! |---|---|
//! | `0x000..0x3F8` | the body (at most 254 words), falls into the tail |
//! | `0x400..0x580` | the six near callees, 16 words each |
//! | `0x800..0x8E0` | the in-page pool: literals, callee and device addresses, patch words |
//! | `0xF00..0xFC0` | the tail: reads and rewrites a literal of the next page, then `b SPIN` |
//! | `0xFFC` | `SPIN`: `b .` |
//! | `0x1000..0x1040` | the next page (`POOL`, `r9`): literals the tail reads and stores over |
//! | `0x2000..0x2100` | four far callees, a page away |

use super::asm::{self as a, AL};
use super::blocks::Gen;
use super::faults::guarded_fault;
#[cfg(test)]
use super::faults::is_fault_word;
use super::parts::{callees, straight_word, tail, with_cond};
use super::CODE;
use crate::machine::sb::testkit::state::{attach_boards, DATA_A, DATA_B, STACK};
use crate::machine::sb::testkit::Rng;
use crate::machine::sysbus::{GPIO_BASE, PORT1_BASE, PORT2_BASE, TICK_ADDRESS};
use crate::machine::Machine;

/// Near callees (16 words each).
pub(super) const FUNC_AT: [u32; 6] = [
    CODE + 0x400,
    CODE + 0x440,
    CODE + 0x480,
    CODE + 0x4C0,
    CODE + 0x500,
    CODE + 0x540,
];
/// Far callees: a page and more away from the body.
pub(super) const FAR_AT: [u32; 4] = [CODE + 0x2000, CODE + 0x2040, CODE + 0x2080, CODE + 0x20C0];
/// The in-page pool.
pub(super) const LIT_AT: u32 = CODE + 0x800;
/// The tail the body falls into.
pub(super) const TAIL_AT: u32 = CODE + 0xF00;
/// `b .`, where every program ends.
pub(super) const SPIN_AT: u32 = CODE + 0xFFC;
/// The next page: `r9` points here, the tail reads and rewrites its literals.
pub(super) const POOL: u32 = CODE + 0x1000;
/// Words of the body at most (the callees start 0x400 bytes in; room for the jump to the tail).
pub(super) const BODY_MAX: usize = 0x400 / 4 - 2;

/// Pool layout: literal words, callee addresses, device addresses, patch words.
pub(super) const FUNC_IDX: usize = 16;
pub(super) const DEV_IDX: usize = 24;
pub(super) const PATCH_IDX: usize = 40;
const POOL_WORDS: usize = 56;

/// Device addresses of the pool (`DEV_IDX..`): the first eight lie in poll windows (a poll loop
/// on them ends once the poll breaker substitutes values), then the tick, the GPIO set/clear
/// registers, the strobe, a padded DSP port, and two path 1 ports that complete a message with
/// two stores. ASSUMED: addresses of `sysbus.rs`/`panel.rs`/`i2c.rs`/`uart.rs`; checked by
/// `the_device_table_hits_every_device_model`.
pub(super) const DEV: [u32; 16] = [
    0x01C0_0010,
    0x01C0_0020,
    0x01D0_D014,
    0x01C2_2008,
    0x6000_0010,
    0x6000_0020,
    0xB000_0010,
    0x6000_0100,
    TICK_ADDRESS,
    TICK_ADDRESS + 4,
    GPIO_BASE + 0x18,
    GPIO_BASE + 0x1C,
    0x6000_0015,
    PORT2_BASE + 0x10,
    PORT1_BASE,
    PORT1_BASE + 0x10,
];

/// A program image and the state it starts from.
#[derive(Clone, Debug)]
pub(crate) struct Program {
    /// Words placed in memory: `(address, words)`.
    pub segments: Vec<(u32, Vec<u32>)>,
    pub entry: u32,
    /// `r0..r14` at the start.
    pub regs: [u32; 15],
    /// Initial flag nibble (`n << 3 | z << 2 | c << 1 | v`); `None` keeps the reset flags.
    pub nzcv: Option<u8>,
    /// Attach the panel, I2C and UART2 models.
    pub boards: bool,
    /// Seed of the biased words that fill the data pages and the stack; `None` leaves them zero.
    pub fill: Option<u64>,
    /// Ranges mapped before the run (the device pages: the poll breaker only writes mapped words).
    pub maps: Vec<(u32, u32)>,
    /// Addresses worth stopping at or hooking: callee entries and body words.
    pub marks: Vec<u32>,
}

impl Program {
    /// The scaffold image: `words` at [`CODE`], random `r0..r7` from `seed`, `r8`/`r9` pointing
    /// at data, `r10` into the code, a stack, `lr` and `pc` at the code. Bit for bit the machine
    /// `testgen::machine_with` always built.
    pub fn flat(words: &[u32], seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let mut regs = [0; 15];
        for r in regs.iter_mut().take(8) {
            *r = rng.next();
        }
        (regs[8], regs[9], regs[10], regs[13], regs[14]) =
            (DATA_A, DATA_B, CODE + 0x100, STACK, CODE);
        Self {
            segments: vec![(CODE, words.to_vec())],
            entry: CODE,
            regs,
            nzcv: None,
            boards: false,
            fill: None,
            maps: Vec::new(),
            marks: (0..words.len().min(64) as u32)
                .map(|i| CODE + 4 * i)
                .collect(),
        }
    }

    /// The words of the segment that starts at `at`.
    pub fn segment(&self, at: u32) -> &[u32] {
        self.segments
            .iter()
            .find(|s| s.0 == at)
            .map_or(&[], |s| &s.1)
    }
}

/// The machine `p` starts on: segments written, registers and flags set, data filled, device
/// pages mapped, boards attached; `predecode` selects tier 1 or the plain interpreter.
pub(crate) fn machine_from(p: &Program, predecode: bool) -> Machine {
    let mut m = Machine::new();
    m.predecode = predecode;
    for (at, words) in &p.segments {
        for (i, w) in words.iter().enumerate() {
            m.bus.mem.write32(at + 4 * i as u32, *w);
        }
    }
    for (base, size) in &p.maps {
        m.bus.mem.map(*base, *size);
    }
    m.cpu.r[..15].copy_from_slice(&p.regs);
    m.cpu.r[15] = p.entry;
    if let Some(f) = p.nzcv {
        (m.cpu.n, m.cpu.z, m.cpu.c, m.cpu.v) = (f & 8 != 0, f & 4 != 0, f & 2 != 0, f & 1 != 0);
    }
    if let Some(seed) = p.fill {
        let mut rng = Rng::new(seed);
        for page in [DATA_A, DATA_B, STACK] {
            for i in 0..1024 {
                m.bus.mem.write32(page + 4 * i, rng.biased());
            }
        }
    }
    if p.boards {
        attach_boards(&mut m.bus);
    }
    m
}

/// The in-page pool: 16 literals, callee addresses, device addresses, 16 patch words.
fn pool(rng: &mut Rng) -> Vec<u32> {
    let mut w: Vec<u32> = (0..16).map(|_| rng.biased()).collect();
    w.extend(FUNC_AT.iter().chain(&FAR_AT[..2]));
    w.extend(DEV);
    for _ in 0..16 {
        let word = straight_word(rng);
        w.push(if rng.chance(5) {
            with_cond(word, rng.below(13))
        } else if rng.chance(8) {
            guarded_fault(rng) // a store can copy it over the program
        } else {
            word
        });
    }
    debug_assert_eq!(w.len(), POOL_WORDS);
    w
}

/// A structured program with a body of about `words` words (16 to 140): see the module header.
/// Every loop ends, every access stays in the map, and the program ends spinning at `SPIN_AT`
/// unless a patch store or an undefined word changes its course.
pub(crate) fn structured_program(rng: &mut Rng, words: usize) -> Program {
    let target = words.clamp(16, 140);
    let mut g = Gen::new(rng);
    g.straight(2);
    while g.w.len() < target {
        g.block();
    }
    g.jump(AL, TAIL_AT);
    let Gen {
        w: mut body,
        patchable,
        fixups,
        ..
    } = g;
    assert!(body.len() <= BODY_MAX, "body of {} words", body.len());
    for i in fixups {
        let n = rng.pick(&patchable) as i32;
        body[i] = a::str(3, 10, 4 * n);
    }
    let mut segments = vec![(CODE, body), (LIT_AT, pool(rng))];
    let mut marks: Vec<u32> = (0..8)
        .map(|_| CODE + 4 * rng.below(target as u32))
        .collect();
    for (at, f) in callees(rng) {
        marks.push(at);
        segments.push((at, f));
    }
    segments.push((TAIL_AT, tail(rng)));
    segments.push((SPIN_AT, vec![a::branch(AL, false, SPIN_AT, SPIN_AT)]));
    segments.push((POOL, (0..16).map(|_| rng.biased()).collect()));
    let mut regs = [0; 15];
    for r in regs.iter_mut().take(8) {
        *r = rng.biased();
    }
    (regs[8], regs[9], regs[10], regs[13], regs[14]) = (DATA_A, POOL, CODE, STACK, SPIN_AT);
    let mut maps: Vec<(u32, u32)> = DEV.iter().map(|d| (d & !0xFFF, 0x1000)).collect();
    maps.sort_unstable();
    maps.dedup();
    Program {
        segments,
        entry: CODE,
        regs,
        nzcv: Some(rng.below(16) as u8),
        boards: false,
        fill: Some(rng.next_u64()),
        maps,
        marks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The machine the scaffold's `machine_with` built, as it was written.
    fn scaffold_machine(words: &[u32], seed: u64, predecode: bool) -> Machine {
        let mut rng = Rng::new(seed);
        let mut m = Machine::new();
        m.predecode = predecode;
        for (i, w) in words.iter().enumerate() {
            m.bus.mem.write32(CODE + 4 * i as u32, *w);
        }
        for reg in m.cpu.r.iter_mut().take(8) {
            *reg = rng.next();
        }
        (m.cpu.r[8], m.cpu.r[9], m.cpu.r[10], m.cpu.r[13]) =
            (0x9000_0000, 0x9000_1000, CODE + 0x100, 0x9000_2000);
        m.cpu.r[14] = CODE;
        m.cpu.r[15] = CODE;
        m
    }

    #[test]
    fn the_flat_image_is_the_scaffold_machine_bit_for_bit() {
        for seed in 0..8 {
            let words = super::super::random_program(&mut Rng::new(seed), 40);
            let (a, b) = (
                scaffold_machine(&words, seed, true),
                machine_from(&Program::flat(&words, seed), true),
            );
            assert_eq!(
                (a.cpu.r, a.cpu.cpsr(), a.state_hash()),
                (b.cpu.r, b.cpu.cpsr(), b.state_hash())
            );
            assert_eq!(a.bus.mem.auto_mapped, b.bus.mem.auto_mapped);
        }
    }

    #[test]
    fn programs_are_deterministic_per_seed_and_laid_out_in_the_map() {
        let (p, q) = (
            structured_program(&mut Rng::new(7), 80),
            structured_program(&mut Rng::new(7), 80),
        );
        assert_eq!(
            (&p.segments, p.regs, p.nzcv, p.fill),
            (&q.segments, q.regs, q.nzcv, q.fill)
        );
        assert_ne!(
            p.segments,
            structured_program(&mut Rng::new(8), 80).segments
        );
        let mut spans: Vec<(u32, u32)> = p
            .segments
            .iter()
            .map(|(at, w)| (*at, at + 4 * w.len() as u32))
            .collect();
        spans.sort_unstable();
        for pair in spans.windows(2) {
            assert!(pair[0].1 <= pair[1].0, "segments overlap: {pair:x?}");
        }
        assert_eq!(p.segment(SPIN_AT), &[0xEAFF_FFFE], "b .");
        assert_eq!(p.segment(LIT_AT).len(), POOL_WORDS);
        assert_eq!(p.segment(LIT_AT)[FUNC_IDX], FUNC_AT[0]);
        assert_eq!(p.segment(LIT_AT)[DEV_IDX + 8], TICK_ADDRESS);
        assert_eq!((p.regs[9], p.regs[14]), (POOL, SPIN_AT));
        let last = *p.segment(CODE).last().unwrap();
        assert_eq!(
            last,
            a::branch(
                AL,
                false,
                CODE + 4 * (p.segment(CODE).len() as u32 - 1),
                TAIL_AT
            )
        );
    }

    #[test]
    fn the_pool_holds_a_fault_word_among_its_patch_words_now_and_then() {
        let mut with = 0;
        for seed in 0..200 {
            let w = pool(&mut Rng::new(seed));
            assert_eq!(w.len(), POOL_WORDS);
            with += u32::from(w[PATCH_IDX..].iter().any(|w| is_fault_word(*w)));
        }
        assert!(
            (80..=190).contains(&with),
            "{with} of 200 pools hold a fault word to be patched in"
        );
    }

    #[test]
    fn every_size_of_body_fits_its_page() {
        for (seed, words) in (0..60).zip((0..).step_by(5)) {
            let p = structured_program(&mut Rng::new(seed), words);
            let n = p.segment(CODE).len();
            assert!((17..=BODY_MAX).contains(&n), "seed {seed}: {n} words");
        }
    }

    #[test]
    fn the_device_table_hits_every_device_model() {
        let mut m = machine_from(&structured_program(&mut Rng::new(1), 30), false);
        m.bus.pc = CODE;
        for d in &DEV[..8] {
            assert!(
                m.bus.has_side_effect(*d, false),
                "{d:#x} is in a poll window"
            );
        }
        assert!(
            m.bus.has_side_effect(TICK_ADDRESS, false)
                && m.bus.has_side_effect(GPIO_BASE + 0x18, true)
        );
        assert!(
            m.bus.has_side_effect(PORT1_BASE, true)
                && m.bus.has_side_effect(PORT2_BASE + 0x10, true)
        );
        assert!(
            DEV.iter().all(|d| m.bus.mem.is_mapped(*d)),
            "the device pages are mapped before the run"
        );
    }
}
