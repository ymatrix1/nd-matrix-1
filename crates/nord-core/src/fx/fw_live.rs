//! Context export of the Mod 1 / Mod 2 firmware ports for the live FX oracle
//! (`docs/research/fx-vs-oracle.md`, `tools/fx_vs_oracle.py`).
//!
//! The ports keep the routine context, the delay rings and the scratch words as arrays at the
//! firmware's own data addresses (`fw_chorus::CTX_I1`, `RING_BASE`, `fw_mod2_run::RING2_BASE`).
//! [`live_install`] returns those words as `(address, word)` pairs plus the routine's pc on FX
//! Core 1, so the SHARC oracle can run the *same* routine with the *same* words on a live signal
//! and the output compares with the port sample by sample. Offline tooling only: it allocates and
//! is never called from the audio path.
//!
//! Provenance: the pcs are the core 2 pcs of the `Routine` table plus `0x974`
//! (`docs/research/fx-type-map.md` s2, the core 1 column; VERIFIED there by the descriptor lists).

use super::fw_chorus::{CTX_I1, CTX_LO, RING_BASE};
use super::fw_mod1::{FwMod1Stage, Mod1Shape};
use super::fw_mod1_rest::FwMod1Rest;
use super::fw_mod2::FwMod2Stage;
use super::fw_mod2_law_stage::FwLawStage;
use super::fw_mod2_run::{Mod2State, Routine, RING2_BASE};
use super::fw_vibe_stage::FwVibeStage;
use crate::organ::FIRMWARE_FRAME_RATE_HZ;

/// Words that make one oracle instance of an FX routine: the context around `i1`, the rings.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveInstall {
    /// Routine entry on FX Core 1.
    pub pc: u32,
    /// Context pointer `I1` the dispatcher passes in the stage record's state word.
    pub i1: u32,
    /// `(data address, word)` of the context, the delay rings and their zero fill.
    pub words: Vec<(u32, u32)>,
}

/// Core 1 pc of a routine: the core 2 pc of [`Routine`] plus `0x974`.
const fn core1_pc(r: Routine) -> u32 {
    0x974
        + match r {
            Routine::Vibe => 0x123C84,
            Routine::Flang => 0x12348D,
            Routine::Phas0 => 0x12379B,
            Routine::Phas1 => 0x1238FC,
            Routine::Ens => 0x123A7B,
            Routine::Spin => 0x123557,
            Routine::Rm => 0x1239D0,
            Routine::Wah => 0x123DD2,
            Routine::Awah => 0x123E9B,
            Routine::Pump => 0x122D2C,
        }
}

/// Context, ring and (SPIN) second ring of a [`Mod2State`] as words; the block count word is
/// set to 2 (one live pass = 4 frames = 2 blocks of two frames).
fn from_state(routine: Routine, st: &Mod2State) -> Vec<(u32, u32)> {
    let mut ctx = st.ctx;
    ctx[(routine.blocks_offset() + 0x20) as usize] = 2;
    let mut w: Vec<(u32, u32)> = ctx.iter().zip(CTX_LO..).map(|(&v, a)| (a, v)).collect();
    w.extend(st.ring.iter().zip(RING_BASE..).map(|(&v, a)| (a, v)));
    if routine == Routine::Spin {
        w.extend(st.ring2.iter().zip(RING2_BASE..).map(|(&v, a)| (a, v)));
    }
    w
}

/// The install words of the port for FX slot `slot` (1 = Mod 1, 2 = Mod 2, 3 = compressor with
/// `amount` = `kCmpAmt`, type 0) type `ty`, rate and
/// amount in `0..=127`, at the firmware frame rate (no rate scaling). `None` for a type without
/// a port with this export (Mod 1 types 12 and up, Mod 2 types 12 and up).
pub fn live_install(slot: u8, ty: u8, rate: u32, amount: u32) -> Option<LiveInstall> {
    let fs = FIRMWARE_FRAME_RATE_HZ as f32;
    let (routine, words) = match (slot, ty) {
        (2, 4 | 5) => {
            let st = FwMod2Stage::new(ty == 5, rate, amount, fs);
            let core = st.core();
            let kind = core.kind();
            let mut ctx = *core.words();
            ctx[(kind.blocks_offset() + 0x20) as usize] = 2;
            let mut w: Vec<(u32, u32)> = ctx.iter().zip(CTX_LO..).map(|(&v, a)| (a, v)).collect();
            w.extend(core.ring().iter().zip(RING_BASE..).map(|(&v, a)| (a, v)));
            let pc = if ty == 5 { 0x1238EB } else { 0x123798 };
            return Some(LiveInstall {
                pc,
                i1: CTX_I1,
                words: w,
            });
        }
        (2, 6 | 7) => {
            let st = FwVibeStage::new(rate, amount, fs);
            (Routine::Vibe, from_state(Routine::Vibe, st.state()))
        }
        (2, _) => {
            let st = FwLawStage::new(ty, rate, amount, fs)?;
            let (r, s) = st.state();
            (r, from_state(r, s))
        }
        (3, 0) => {
            // The compressor `0x120630` (the same pc on both cores): 4 frames per call, M3 / M4
            // are the frame offsets in the context.
            let c = super::fw_params::compressor_for_amount(amount as f32);
            let words = c
                .words()
                .iter()
                .zip(CTX_LO..)
                .map(|(&w, a)| (a, w))
                .collect();
            return Some(LiveInstall {
                pc: 0x120630,
                i1: CTX_I1,
                words,
            });
        }
        (1, 0..=3) => {
            let st = FwMod1Stage::new(
                if ty < 2 {
                    Mod1Shape::AutoPan
                } else {
                    Mod1Shape::Tremolo
                },
                ty & 1 == 1,
                rate,
                amount,
                fs,
            );
            let (pc, ctx) = st.live_words();
            let mut words: Vec<(u32, u32)> = ctx
                .iter()
                .map(|&(o, w)| (CTX_I1.wrapping_add_signed(o), w))
                .collect();
            words.extend((0..0x410).map(|k| (RING_BASE + k, 0)));
            return Some(LiveInstall {
                pc,
                i1: CTX_I1,
                words,
            });
        }
        (1, 4..=11) => {
            let st = FwMod1Rest::new(ty, rate, amount, fs)?;
            let (r, s) = st.state();
            (r, from_state(r, s))
        }
        _ => return None,
    };
    Some(LiveInstall {
        pc: core1_pc(routine),
        i1: CTX_I1,
        words,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_wired_type_exports_a_context_with_two_blocks() {
        for (slot, ty) in (0..12u8).map(|t| (2, t)).chain((0..12).map(|t| (1, t))) {
            let li = live_install(slot, ty, 64, 64).unwrap_or_else(|| panic!("{slot}/{ty}"));
            assert_eq!(li.i1, CTX_I1);
            assert!(
                li.pc >= 0x123000 && li.pc < 0x125000,
                "{slot}/{ty}: {:#x}",
                li.pc
            );
            assert!(li.words.len() >= 0x20 + 0x410, "{slot}/{ty}");
        }
    }

    #[test]
    fn the_compressor_exports_its_26_words_at_the_shared_context_base() {
        let c = live_install(3, 0, 0, 64).unwrap();
        assert_eq!((c.pc, c.i1, c.words.len()), (0x120630, CTX_I1, 26));
        assert_eq!(c.words[0].0, CTX_I1 - 0x20);
        let loud = live_install(3, 0, 0, 127).unwrap();
        assert_ne!(c.words, loud.words, "the amount reaches the words");
        assert!(live_install(3, 1, 0, 0).is_none());
    }

    #[test]
    fn unwired_slots_and_types_return_none() {
        assert!(live_install(1, 12, 0, 0).is_none());
        assert!(live_install(1, 255, 0, 0).is_none());
        assert!(live_install(3, 4, 0, 0).is_none());
        assert!(live_install(2, 12, 0, 0).is_none());
    }

    #[test]
    fn chorus_pcs_and_the_block_word_match_the_oracle_harness() {
        let c4 = live_install(2, 4, 64, 64).unwrap();
        assert_eq!(c4.pc, 0x123798);
        let blocks = c4.words.iter().find(|&&(a, _)| a == CTX_I1 + 9).unwrap().1;
        assert_eq!(blocks, 2);
        assert_eq!(live_install(2, 5, 64, 64).unwrap().pc, 0x1238EB);
        // `fx_oracle_live.py` chor4: delay word c[-0x20] = 12800 + 4096 rate at rate 64.
        let rate = c4
            .words
            .iter()
            .find(|&&(a, _)| a == CTX_I1 - 0x20)
            .unwrap()
            .1;
        assert_eq!(rate, 12800 + 4096 * 64);
    }

    #[test]
    fn spin_carries_its_second_ring_and_the_core1_pcs_are_distinct() {
        let spin = live_install(2, 10, 64, 64).unwrap();
        assert!(spin.words.iter().any(|&(a, _)| a == RING2_BASE));
        let mut pcs: Vec<u32> = (0..12u8)
            .filter(|t| !matches!(t, 4 | 5))
            .map(|t| live_install(2, t, 1, 1).unwrap().pc)
            .collect();
        pcs.sort_unstable();
        pcs.dedup();
        assert_eq!(pcs.len(), 6, "PHAS 0, PHAS 1, FLANG, VIBE, ENS, SPIN");
    }
}
