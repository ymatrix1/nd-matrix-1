//! Predecoded ARM instructions: the decoder that classifies a word once, the cache entry, and the
//! one-instruction step that runs an entry.
//!
//! A cached run fetches an [`Entry`] by `pc` instead of reading and decoding the word each time
//! (`ns4-lle2` follow-up 1). The classification mirrors the `match` chain of `exec_arm_body`
//! (`arm.rs`): anything not on the list of hot classes gets [`op::SLOW`] and runs through that
//! function unchanged, so a class that is not specialised can only be slow, never different.
//! The differential test at the bottom runs random words through both paths.

use super::arm_cached::{Handler, Table};
use super::{Bus, Cpu, Fault};

/// Entries per 4 KiB page of ARM code.
pub const ENTRIES: usize = 1024;
/// Number of handler ids ([`op`]).
pub const N_OPS: usize = 16 + 96;
/// Handler table length: a power of two so `op & (TABLE_LEN - 1)` needs no bounds check.
pub const TABLE_LEN: usize = 128;
/// [`Entry::mask`] of an instruction that always runs.
pub const MASK_ALWAYS: u16 = 0xFFFF;

/// Whether condition `cond` (0..=13) holds for flags `nzcv` (`n << 3 | z << 2 | c << 1 | v`).
const fn holds(cond: u32, nzcv: u32) -> bool {
    let (n, z, c, v) = (
        nzcv >> 3 & 1 != 0,
        nzcv >> 2 & 1 != 0,
        nzcv >> 1 & 1 != 0,
        nzcv & 1 != 0,
    );
    match cond {
        0 => z,
        1 => !z,
        2 => c,
        3 => !c,
        4 => n,
        5 => !n,
        6 => v,
        7 => !v,
        8 => c && !z,
        9 => !c || z,
        10 => n == v,
        11 => n != v,
        12 => !z && n == v,
        13 => z || n != v,
        _ => true,
    }
}

const fn cond_masks() -> [u16; 16] {
    let mut t = [0u16; 16];
    let mut cond = 0;
    while cond < 16 {
        let mut f = 0;
        while f < 16 {
            if holds(cond, f) {
                t[cond as usize] |= 1 << f;
            }
            f += 1;
        }
        cond += 1;
    }
    t
}

/// Bit `nzcv` of `COND_MASK[cond]` is set when the condition holds (14 and 15: always).
pub const COND_MASK: [u16; 16] = cond_masks();

/// Handler ids.
pub mod op {
    pub const UNDECODED: u8 = 0;
    pub const SLOW: u8 = 1;
    pub const UNCOND: u8 = 2;
    pub const B: u8 = 3;
    pub const BL: u8 = 4;
    pub const BX: u8 = 5;
    pub const LDR: u8 = 6;
    pub const LDRB: u8 = 7;
    pub const STR: u8 = 8;
    pub const STRB: u8 = 9;
    pub const LDM: u8 = 10;
    pub const HALF: u8 = 11;
    pub const MUL: u8 = 12;
    /// First data-processing id; `dp(form, opcode, s)` = `DP_BASE + form * 32 + opcode * 2 + s`.
    pub const DP_BASE: u8 = 16;

    pub const fn dp(form: u32, opcode: u32, s: bool) -> u8 {
        DP_BASE + (form * 32 + opcode * 2 + s as u32) as u8
    }
}

/// One decoded instruction word.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub insn: u32,
    /// `COND_MASK[cond]`; [`MASK_ALWAYS`] runs without testing the flags.
    pub mask: u16,
    /// Handler id ([`op`]).
    pub op: u8,
    /// Straight-line instructions starting here before the next block ender (see
    /// [`ends_block`]), capped at 255 and at the end of the page. `0` for an ender itself.
    pub run: u8,
}

/// A page of decoded ARM code.
pub type ArmPage = [Entry; ENTRIES];

/// Classify `insn`.
pub fn decode_arm(insn: u32) -> Entry {
    let cond = (insn >> 28) as u8;
    if cond == 0xF {
        return Entry {
            insn,
            mask: MASK_ALWAYS,
            op: op::UNCOND,
            run: 0,
        };
    }
    let id = match insn >> 25 & 7 {
        0 => decode_000(insn),
        1 => decode_dp_imm(insn),
        2 | 3 => {
            if insn >> 25 & 1 != 0 && insn >> 4 & 1 != 0 {
                op::SLOW // undefined media space
            } else {
                match (insn >> 20 & 1 != 0, insn >> 22 & 1 != 0) {
                    (true, false) => op::LDR,
                    (true, true) => op::LDRB,
                    (false, false) => op::STR,
                    (false, true) => op::STRB,
                }
            }
        }
        4 => op::LDM,
        5 => {
            if insn >> 24 & 1 != 0 {
                op::BL
            } else {
                op::B
            }
        }
        _ => op::SLOW,
    };
    Entry {
        insn,
        mask: COND_MASK[cond as usize],
        op: id,
        run: 0,
    }
}

/// Test, compare and S-less opcodes 8..=11 are the "miscellaneous" hole of data processing.
fn is_misc(insn: u32) -> bool {
    (8..=11).contains(&(insn >> 21 & 0xF)) && insn >> 20 & 1 == 0
}

fn decode_000(insn: u32) -> u8 {
    if insn >> 4 & 1 != 0 && insn >> 7 & 1 != 0 {
        // multiply, swap, halfword transfers
        return if insn >> 5 & 3 != 0 {
            op::HALF
        } else if insn >> 24 & 1 != 0 {
            op::SLOW // swp
        } else {
            op::MUL
        };
    }
    if is_misc(insn) {
        // BX Rm: opcode 9 (CMN without S), sub-opcode 1
        return if insn >> 21 & 0xF == 9 && insn >> 4 & 0xF == 1 {
            op::BX
        } else {
            op::SLOW
        };
    }
    let form = Cpu::dp_form(insn);
    op::dp(form, insn >> 21 & 0xF, insn >> 20 & 1 != 0)
}

fn decode_dp_imm(insn: u32) -> u8 {
    if is_misc(insn) {
        return op::SLOW; // msr immediate or undefined
    }
    op::dp(0, insn >> 21 & 0xF, insn >> 20 & 1 != 0)
}

/// True when the instruction may change `pc` other than to the next word, change the instruction
/// set, or fault: branches, loads and data processing into `r15`, `LDM` with `pc` in the list,
/// multiplies into `r15`, and every instruction left to the generic `exec_arm_body`. The cached
/// run executes everything before an ender without looking at `jumped` or the fault.
pub fn ends_block(e: &Entry) -> bool {
    let insn = e.insn;
    let rd = insn >> 12 & 0xF;
    let load = insn >> 20 & 1 != 0;
    match e.op {
        op::LDR | op::LDRB => rd == 15,
        op::STR | op::STRB => false,
        op::LDM => load && insn >> 15 & 1 != 0,
        op::HALF => load && rd == 15,
        op::MUL => rd == 15 || insn >> 16 & 0xF == 15,
        id if id >= op::DP_BASE => !(8..=11).contains(&(insn >> 21 & 0xF)) && rd == 15,
        _ => true, // B, BL, BX, UNCOND, SLOW
    }
}

/// Decode a whole page of ARM code and fill the `run` lengths. The last word of a page is always
/// an ender (the next word lies on another page).
pub fn build_page(words: &[u32; ENTRIES]) -> Box<ArmPage> {
    let mut page = Box::new([Entry::default(); ENTRIES]);
    for (slot, w) in page.iter_mut().zip(words) {
        *slot = decode_arm(*w);
    }
    let mut next_run = 0u8;
    for i in (0..ENTRIES).rev() {
        let ender = i == ENTRIES - 1 || ends_block(&page[i]);
        page[i].run = if ender { 0 } else { next_run.saturating_add(1) };
        next_run = page[i].run;
    }
    page
}

impl Cpu {
    /// Run one decoded ARM instruction at `pc`; `true` means it faulted (fault in `self.fault`).
    /// Same effect as [`Cpu::step`] on the word the entry was decoded from, except that the
    /// instruction counter is the caller's business (the cached loop counts in a local).
    #[inline(always)]
    pub(crate) fn exec_decoded<B: Bus + 'static>(
        &mut self,
        bus: &mut B,
        e: Entry,
        pc: u32,
    ) -> bool {
        let table: &'static [Handler<B>; TABLE_LEN] = Table::<B>::ARM;
        self.jumped = false;
        self.r[15] = pc.wrapping_add(8);
        let fault = if e.mask == MASK_ALWAYS || e.mask >> self.nzcv() & 1 != 0 {
            table[e.op as usize & (TABLE_LEN - 1)](self, bus, e.insn)
        } else {
            false
        };
        if !self.jumped {
            self.r[15] = pc.wrapping_add(if self.thumb { 2 } else { 4 });
        }
        fault
    }

    /// Run a decoded instruction that is not a block ender ([`ends_block`]): it cannot jump or
    /// fault, so `pc` simply moves on to the next word (the caller keeps it).
    #[inline(always)]
    pub(crate) fn exec_pure<B: Bus + 'static>(&mut self, bus: &mut B, e: Entry, pc: u32) {
        let table: &'static [Handler<B>; TABLE_LEN] = Table::<B>::ARM;
        self.r[15] = pc.wrapping_add(8);
        if e.mask == MASK_ALWAYS || e.mask >> self.nzcv() & 1 != 0 {
            table[e.op as usize & (TABLE_LEN - 1)](self, bus, e.insn);
        }
    }

    /// [`Cpu::exec_decoded`] counting the instruction and returning the fault (tests, probes).
    #[allow(dead_code)]
    pub(crate) fn step_decoded<B: Bus + 'static>(
        &mut self,
        bus: &mut B,
        e: Entry,
        pc: u32,
    ) -> Result<(), Fault> {
        self.insns += 1;
        if self.exec_decoded(bus, e, pc) {
            return Err(self.fault.take().expect("faulting handler sets the fault"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 16) as u32
        }
    }

    /// Memory that "contains" a deterministic byte at every address and logs every access, so two
    /// runs can be compared without cloning a 4 GiB address space.
    #[derive(Clone, Default)]
    struct TestBus {
        written: std::collections::BTreeMap<u32, u8>,
        log: Vec<(u8, u32, u32)>,
    }

    impl TestBus {
        fn byte(&self, a: u32) -> u8 {
            *self
                .written
                .get(&a)
                .unwrap_or(&((a.wrapping_mul(0x9E37_79B1) >> 24) as u8))
        }
    }

    impl Bus for TestBus {
        fn read8(&mut self, a: u32) -> u8 {
            self.log.push((0, a, 0));
            self.byte(a)
        }
        fn read16(&mut self, a: u32) -> u16 {
            self.log.push((1, a, 0));
            u16::from_le_bytes([self.byte(a), self.byte(a.wrapping_add(1))])
        }
        fn read32(&mut self, a: u32) -> u32 {
            self.log.push((2, a, 0));
            u32::from_le_bytes([0, 1, 2, 3].map(|i| self.byte(a.wrapping_add(i))))
        }
        fn write8(&mut self, a: u32, v: u8) {
            self.log.push((3, a, v as u32));
            self.written.insert(a, v);
        }
        fn write16(&mut self, a: u32, v: u16) {
            self.log.push((4, a, v as u32));
            for (i, b) in v.to_le_bytes().iter().enumerate() {
                self.written.insert(a.wrapping_add(i as u32), *b);
            }
        }
        fn write32(&mut self, a: u32, v: u32) {
            self.log.push((5, a, v));
            for (i, b) in v.to_le_bytes().iter().enumerate() {
                self.written.insert(a.wrapping_add(i as u32), *b);
            }
        }
        fn fetch32(&mut self, a: u32) -> u32 {
            u32::from_le_bytes([0, 1, 2, 3].map(|i| self.byte(a.wrapping_add(i))))
        }
    }

    /// Run `insn` at `0x1000` from one random state down both paths; returns the diverging
    /// description or `None`.
    fn diverges(insn: u32, rng: &mut Rng) -> Option<String> {
        let mut cpu = Cpu::new();
        for r in cpu.r.iter_mut() {
            *r = match rng.next() % 4 {
                0 => 0x2000 + (rng.next() % 0x100),
                1 => rng.next() % 40,
                2 => 0x2000 + 4 * (rng.next() % 64),
                _ => rng.next(),
            };
        }
        cpu.r[15] = 0x1000;
        cpu.set_cpsr(rng.next() & 0xF800_0000 | 0x13);
        cpu.spsr = rng.next();
        let (mut ca, mut cb) = (cpu.clone(), cpu);
        let (mut bus_a, mut bus_b) = (TestBus::default(), TestBus::default());
        for bus in [&mut bus_a, &mut bus_b] {
            for (i, b) in insn.to_le_bytes().iter().enumerate() {
                bus.written.insert(0x1000 + i as u32, *b);
            }
        }
        let ra = ca.step(&mut bus_a);
        let rb = cb.step_decoded(&mut bus_b, decode_arm(insn), 0x1000);
        let same = ra == rb
            && ca.r == cb.r
            && ca.cpsr() == cb.cpsr()
            && (ca.spsr, ca.jumped, ca.insns, ca.cp15_ctrl)
                == (cb.spsr, cb.jumped, cb.insns, cb.cp15_ctrl)
            && bus_a.written == bus_b.written
            && bus_a.log == bus_b.log;
        (!same).then(|| {
            format!(
                "insn {insn:#010x} op {}: {ra:?} vs {rb:?}",
                decode_arm(insn).op
            )
        })
    }

    #[test]
    fn a_decoded_step_equals_the_plain_step_for_random_words() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut classes = [0u32; N_OPS];
        for n in 0..200_000u32 {
            let insn = rng.next() ^ (rng.next() << 16);
            // every third word is forced to "always" so conditions do not hide the handlers
            let insn = if n % 3 == 0 {
                insn & 0x0FFF_FFFF | 0xE000_0000
            } else {
                insn
            };
            if let Some(d) = diverges(insn, &mut rng) {
                panic!("{d}");
            }
            classes[decode_arm(insn).op as usize] += 1;
        }
        for id in (op::B..=op::MUL).chain(op::DP_BASE..N_OPS as u8) {
            let (opcode, s) = ((id.wrapping_sub(op::DP_BASE) as u32 % 32) / 2, id & 1 != 0);
            let in_misc_hole = id >= op::DP_BASE && (8..=11).contains(&opcode) && !s;
            assert!(
                in_misc_hole || classes[id as usize] > 0,
                "class {id} never generated"
            );
        }
    }

    #[test]
    fn cond_mask_table_matches_the_flag_logic_for_all_flags() {
        for cond in 0..14u32 {
            for f in 0..16u32 {
                let mut c = Cpu::new();
                (c.n, c.z, c.c, c.v) = (f & 8 != 0, f & 4 != 0, f & 2 != 0, f & 1 != 0);
                assert_eq!(c.nzcv(), f);
                assert_eq!(
                    COND_MASK[cond as usize] >> f & 1 != 0,
                    c.cond(cond),
                    "{cond} {f}"
                );
            }
        }
        assert_eq!(COND_MASK[14], MASK_ALWAYS);
    }

    #[test]
    fn decode_picks_the_hot_classes() {
        assert_eq!(decode_arm(0xE3A0_0005).op, op::dp(0, 13, false)); // mov r0,#5
        assert_eq!(decode_arm(0xE150_0001).op, op::dp(1, 10, true)); // cmp r0,r1
        assert_eq!(decode_arm(0xE1A0_0211).op, op::dp(2, 13, false)); // mov r0,r1,lsl r2
        assert_eq!(decode_arm(0xEB00_0001).op, op::BL);
        assert_eq!(decode_arm(0xEAFF_FFFE).op, op::B);
        assert_eq!(decode_arm(0xE12F_FF1E).op, op::BX);
        assert_eq!(decode_arm(0xE591_0000).op, op::LDR);
        assert_eq!(decode_arm(0xE5C1_0000).op, op::STRB);
        assert_eq!(decode_arm(0xE92D_4010).op, op::LDM);
        assert_eq!(decode_arm(0xE0C0_00B0).op, op::HALF);
        assert_eq!(decode_arm(0xE001_0092).op, op::MUL);
        assert_eq!(decode_arm(0xE100_0090).op, op::SLOW); // swp
        assert_eq!(decode_arm(0xE10F_3000).op, op::SLOW); // mrs
        assert_eq!(decode_arm(0xF57F_F05F).op, op::UNCOND);
        assert_eq!(decode_arm(0x1A00_0000).mask, COND_MASK[1]);
        assert_eq!(decode_arm(0xE3A0_0005).mask, MASK_ALWAYS);
    }
}
