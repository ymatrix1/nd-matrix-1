//! External-port DMA channel 0 with chain loading, as the SY1SY2 image uses it (D141, opt-in,
//! `Core::enable_epchain`, `sharc-ref` scenario command `epchain`).
//!
//! The SY audio ISR (`0x1C0900..0x1C0B2C`) builds a TCB chain in DM and starts it with
//! `DM(CPEP0) = 0x136008` then `DM(DMAC0) |= 0x125`. This model walks that chain at once (the
//! ISR never waits for completion in the traced paths) against a sparse external memory, and
//! records every transfer so a run can say where the firmware moves data to and from.
//!
//! | Fact | Source | Tag |
//! |---|---|---|
//! | `DMAC0 = 0x180B`, `EIEP0..ICEP0 = 0x1820..0x1825`, `CPEP0 = 0x1826` | ADSP-214xx HRM register table (`.reference/sharc/ADSP-214xx_hwr_rev1.1.pdf`) | VERIFIED |
//! | TCB storage order ascending `CP, EM, EI, C, IM, II`, chain pointer = address of `II`, bits 18:0 address, bit 19 PCI, bit 20 CPDR | HRM listing 4-7 and table 3-13 | VERIFIED |
//! | `CPDR` of the pointer that loads a TCB: 0 = write to internal memory (external read), 1 = internal read (external write), with `DMAC0.OFCEN` set | HRM section "Data Direction on the Fly" | VERIFIED (text); the ISR sets `DMAC0 = ..0x125` (bit 8 = OFCEN) |
//! | The static chain (68 slots x 3 TCBs, 18 words per slot from `DM(0xB6003)`) comes from the ARM init script record at `bank0_CODE@0xC05038BC` (type 4, 1224 words) | `tools/ns4_arm_init_script.py --tcb` | VERIFIED |
//! | A TCB at IOP address `p` sits in normal word DM at `0x80000 + p` | the ISR writes the TCB fields at `0xB6005..` while the chain pointers hold `0x36008..` | EMPIRICAL (consistent for all 204 TCBs) |
//!
//! An external read of an unmapped word leaves the destination unchanged (counted).
//!
//! Not modelled (ASSUMED complete at once): bus timing, `DMAC0` status bits (a `DMAC0` read returns
//! the last written value, so `DEN` never clears and a program that polls for completion would spin), the completion
//! interrupt (`PCI`/`CPDR` interrupt reading of table 3-13 is not used by the traced paths), and
//! any device behind the external bus: external memory is a plain sparse word store.
use super::smpflash::SmpFlash;
use super::Bus;
use std::collections::BTreeMap;

/// Channel 0 control and chain pointer registers (VERIFIED, HRM).
pub const DMAC0: u32 = 0x180B;
pub const CPEP0: u32 = 0x1826;
/// Control bit that arms the channel (`DEN`, VERIFIED: the ISR sets `0x125` after writing `CPEP0`).
pub const DEN: u32 = 1;
/// VERIFIED: bits 18:0 of a chain pointer are the `II` address, bit 19 PCI, bit 20 CPDR (HRM table 3-13).
const ADDR_MASK: u32 = 0x7_FFFF;
const CPDR: u32 = 1 << 20;
/// Normal word DM alias of a TCB IOP address (EMPIRICAL, see the module table).
pub const TCB_ALIAS: u32 = 0x8_0000;
/// Longest chain the model follows. ASSUMED limit, only there to cut loops (the image chain has 204 TCBs).
pub const MAX_TCBS: usize = 1024;
/// Largest transfer the model accepts per TCB. ASSUMED limit (the image uses 1 and 8 words).
pub const MAX_COUNT: u32 = 0x1000;
/// DM window of the SY sample pitch table `DM(0x9354A + index)` (VERIFIED, `sy-pitch-entry-writer.md`).
pub const PITCH_WINDOW: (u32, u32) = (0x9_3500, 0x9_35A0);
/// Transfers kept in [`EpChain::log`]. ASSUMED report limit, no hardware meaning.
pub const LOG_CAP: usize = 8192;

/// One executed TCB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transfer {
    /// IOP address of the TCB's `II` word.
    pub tcb: u32,
    pub ext: u32,
    pub ext_mod: u32,
    pub int: u32,
    pub int_mod: u32,
    pub count: u32,
    /// `true`: external to internal (`CPDR` = 0).
    pub to_dm: bool,
}

impl Transfer {
    /// Whether an internal word of this transfer falls in [`PITCH_WINDOW`].
    pub fn hits_window(&self) -> bool {
        (0..self.count).any(|k| {
            let a = self.int.wrapping_add(self.int_mod.wrapping_mul(k));
            self.to_dm && (PITCH_WINDOW.0..PITCH_WINDOW.1).contains(&a)
        })
    }
}

/// The channel, its sparse external memory and the run totals.
#[derive(Debug, Default)]
pub struct EpChain {
    /// External word memory (a read of a missing word leaves the destination and counts in `unmapped_reads`).
    pub ext: BTreeMap<u32, u32>,
    /// Executed TCBs, first [`LOG_CAP`].
    pub log: Vec<Transfer>,
    pub chains: u64,
    pub tcbs: u64,
    pub words_in: u64,
    pub words_out: u64,
    pub unmapped_reads: u64,
    /// Transfers into DM that touch [`PITCH_WINDOW`].
    pub window_hits: u64,
    /// Chains cut by [`MAX_TCBS`], an oversize count or a TCB outside DM.
    pub aborted: u64,
    /// The SMP sample flash behind banks 1 to 3 (opt-in, `sharc-live --smp` page refill, D191).
    pub flash: Option<SmpFlash>,
    cp: u32,
    dmac: u32,
}

impl EpChain {
    /// Loads little-endian words at external word address `base`.
    pub fn load_words(&mut self, base: u32, words: &[u32]) {
        for (k, &w) in words.iter().enumerate() {
            self.ext.insert(base.wrapping_add(k as u32), w);
        }
    }

    pub fn contains(addr: u32) -> bool {
        addr == DMAC0 || addr == CPEP0
    }

    pub fn read(&self, addr: u32) -> u32 {
        if addr == CPEP0 {
            self.cp
        } else {
            self.dmac
        }
    }

    /// Register write: `CPEP0` stores the pointer, a `DMAC0` write with [`DEN`] runs the chain.
    pub fn write(&mut self, addr: u32, v: u32, dm: &mut impl Bus) {
        if addr == CPEP0 {
            self.cp = v;
        } else {
            self.dmac = v;
            if v & DEN != 0 && self.cp & ADDR_MASK != 0 {
                self.run(dm);
            }
        }
    }

    fn run(&mut self, dm: &mut impl Bus) {
        self.chains += 1;
        let mut cp = self.cp;
        for _ in 0..MAX_TCBS {
            let p = cp & ADDR_MASK;
            if p == 0 {
                self.cp = 0;
                return;
            }
            let at = |k: u32| TCB_ALIAS + p - k;
            let (ii, im, c) = (dm.read32(at(0)), dm.read32(at(1)), dm.read32(at(2)));
            let (ei, em, next) = (dm.read32(at(3)), dm.read32(at(4)), dm.read32(at(5)));
            if c > MAX_COUNT {
                break;
            }
            let t = Transfer {
                tcb: p,
                ext: ei,
                ext_mod: em,
                int: ii,
                int_mod: im,
                count: c,
                to_dm: cp & CPDR == 0,
            };
            self.exec(&t, dm);
            cp = next;
        }
        self.aborted += 1;
        self.cp = 0;
    }

    fn exec(&mut self, t: &Transfer, dm: &mut impl Bus) {
        self.tcbs += 1;
        if t.hits_window() {
            self.window_hits += 1;
        }
        for k in 0..t.count {
            let e = t.ext.wrapping_add(t.ext_mod.wrapping_mul(k));
            let i = t.int.wrapping_add(t.int_mod.wrapping_mul(k));
            if t.to_dm {
                if let Some(w) = self.flash.as_mut().and_then(|f| f.read(e)) {
                    dm.write32(i, w);
                    self.words_in += 1;
                    continue;
                }
                // ASSUMED: a read of an address no device answers leaves the destination as it
                // is (the harness keeps the staging data a `refill` or `stream` model put there)
                match self.ext.get(&e) {
                    Some(&w) => dm.write32(i, w),
                    None => self.unmapped_reads += 1,
                }
                self.words_in += 1;
            } else {
                let w = dm.read32(i);
                if !self.flash.as_mut().is_some_and(|f| f.write(e, w)) {
                    self.ext.insert(e, w);
                }
                self.words_out += 1;
            }
        }
        if self.log.len() < LOG_CAP {
            self.log.push(*t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Mem(BTreeMap<u32, u32>);
    impl Bus for Mem {
        fn read32(&mut self, a: u32) -> u32 {
            self.0.get(&a).copied().unwrap_or(0)
        }
        fn write32(&mut self, a: u32, v: u32) {
            self.0.insert(a, v);
        }
    }

    /// Writes a TCB whose `II` word is at IOP address `p`.
    fn tcb(m: &mut Mem, p: u32, words: [u32; 6]) {
        // words in storage order CP, EM, EI, C, IM, II (ascending addresses)
        for (k, v) in words.into_iter().rev().enumerate() {
            m.0.insert(TCB_ALIAS + p - k as u32, v);
        }
    }

    #[test]
    fn a_two_tcb_chain_writes_a_latch_then_reads_a_block_into_dm() {
        let mut m = Mem::default();
        m.0.insert(0x9_4000, 0x77); // latch value, DM to external (CPDR = 1)
        tcb(&mut m, 0x36008, [0x3_6014, 1, 0x1000, 1, 1, 0x9_4000]);
        tcb(&mut m, 0x36014, [0, 1, 0x2000, 3, 1, 0x9_4100]);
        let mut e = EpChain::default();
        e.load_words(0x2000, &[10, 11, 12]);
        e.write(CPEP0, 0x10_0000 | 0x3_6008, &mut m);
        assert_eq!(e.tcbs, 0, "the chain waits for the enable");
        e.write(DMAC0, 0x125, &mut m);
        assert_eq!(e.ext.get(&0x1000), Some(&0x77));
        assert_eq!(
            [m.0[&0x9_4100], m.0[&0x9_4101], m.0[&0x9_4102]],
            [10, 11, 12]
        );
        assert_eq!((e.chains, e.tcbs, e.words_in, e.words_out), (1, 2, 3, 1));
        assert_eq!(e.read(CPEP0), 0, "chain ended");
        assert_eq!(e.window_hits, 0);
    }

    #[test]
    fn a_read_into_the_pitch_window_is_counted_and_unmapped_reads_leave_dm() {
        let mut m = Mem::default();
        tcb(&mut m, 0x36008, [0, 1, 0x500, 2, 1, 0x9_3549]);
        let mut e = EpChain::default();
        e.write(CPEP0, 0x3_6008, &mut m);
        e.write(DMAC0, DEN, &mut m);
        assert_eq!(e.window_hits, 1);
        assert_eq!(e.unmapped_reads, 2);
        assert_eq!(m.0.get(&0x9_354A), None, "unmapped read leaves DM alone");
        // the same range written to external memory is not a window hit
        let t = Transfer {
            tcb: 0,
            ext: 0,
            ext_mod: 1,
            int: 0x9_3549,
            int_mod: 1,
            count: 2,
            to_dm: false,
        };
        assert!(!t.hits_window());
    }

    #[test]
    fn a_looping_chain_is_cut_and_an_oversize_count_aborts() {
        let mut m = Mem::default();
        tcb(&mut m, 0x36008, [0x3_6008, 1, 0, 1, 1, 0x9_4000]); // points at itself
        let mut e = EpChain::default();
        e.write(CPEP0, 0x3_6008, &mut m);
        e.write(DMAC0, DEN, &mut m);
        assert_eq!((e.tcbs, e.aborted), (MAX_TCBS as u64, 1));
        let mut m2 = Mem::default();
        tcb(&mut m2, 0x36008, [0, 1, 0, MAX_COUNT + 1, 1, 0x9_4000]);
        let mut e2 = EpChain::default();
        e2.write(CPEP0, 0x3_6008, &mut m2);
        e2.write(DMAC0, DEN, &mut m2);
        assert_eq!((e2.tcbs, e2.aborted), (0, 1));
    }

    #[test]
    fn with_a_flash_the_latch_write_and_the_bank_3_read_follow_the_smp_chain() {
        use super::super::smpflash::{SmpFlash, BANK2, BANK3};
        let mut m = Mem::default();
        m.0.insert(0x9_4000, 1); // latch data: flash word = 1 << 24 | low bits
        tcb(&mut m, 0x36008, [0x3_6014, 1, BANK2 + 2, 1, 1, 0x9_4000]);
        tcb(&mut m, 0x36014, [0, 1, BANK3 + 2, 3, 1, 0x9_4100]);
        let mut e = EpChain::default();
        let mut f = SmpFlash::new(vec![0, 1, 0, 2, 0, 3, 0, 4, 0, 5]);
        f.set_base(1 << 24);
        e.flash = Some(f);
        e.write(CPEP0, 0x10_0000 | 0x3_6008, &mut m);
        e.write(DMAC0, 0x125, &mut m);
        assert_eq!(e.flash.as_ref().unwrap().latch(), 1);
        assert_eq!(
            [m.0[&0x9_4100], m.0[&0x9_4101], m.0[&0x9_4102]],
            [3, 4, 5],
            "file words 2..4 at flash words 2..4"
        );
        assert_eq!(
            (e.unmapped_reads, e.ext.len()),
            (0, 0),
            "flash words, not the sparse store"
        );
    }

    #[test]
    fn only_the_two_registers_are_claimed() {
        assert!(EpChain::contains(DMAC0) && EpChain::contains(CPEP0));
        assert!(!EpChain::contains(0x1820) && !EpChain::contains(0x1827));
    }
}
