//! The SMP (piano sample) chip of `sharc-live` (OQ41, D185): constants and the ring tap.
//!
//! The SMP image runs like the SY1SY2 image (no poll loop, FIFO reset interrupt 13, audio
//! interrupt 17, `live_sy::SyModel`), with three differences, all from `docs/research/smp-voice.md`
//! and `piano-bus-convention.md`:
//!
//! | Item | SY1SY2 | SMP | Tag |
//! |---|---|---|---|
//! | Entry after the loader | `0x1C0B2D` | `0x1C11E0` | VERIFIED (`smp_voice_ref` scenarios: `warmup 0x1c11e0 100000 flag12=1`) |
//! | Command FIFO base | `0x93046` | `0x92196` | VERIFIED (`sy-smp-audio.md` s2, `fifo 0x92196`) |
//! | Start-up status word | none | `DM(0x5201) = 0x40` (IOP register, plain memory) | VERIFIED (same scenarios: `poke 0x5201 0x40`) |
//! | Output | frame buffer behind `DM(0x93590)` | post stage ring `0xB2000` / `0xB2014`, 1 stereo frame per interrupt (D200) | EMPIRICAL (listing + `smp_post_stage.scn`, `piano-bus-convention.md` s2b) |
//!
//! The ring holds float words (`bus * 2^-29`) in a 10 word TDM frame: left at `+0`, right at `+1`
//! of each frame, second frame at `+10`. The page ring refill runs through the external port DMA
//! chains of `epchain.rs` against the flash image of `smpflash.rs` (`L`, `F` commands, D191). The voice
//! advances once the command FIFO index carries the ISR bias [`INDEX_BIAS`] and the batch is capped
//! by [`FIFO_CAP`] (D199); the tap then carries varying audio (`docs/research/smp-page-refill.md`).
use crate::core::Core;

/// SMP command FIFO base (`I0` after the FIFO reset interrupt).
pub const FIFO_BASE: u32 = 0x9_2196;
/// VERIFIED (run, clavinet replay, `S` snapshots at `0x1C05CC`: `R3 = 0x80000` in every audio
/// interrupt, `R2 = 0x280` = ring length `L0`): the ISR computes `n = (DM(0x2400) + R3 - I0) >> 1`
/// pairs, so the index word holds `write address - 0x80000`. With the plain address (D185..D191)
/// `n` was `0x3FF..` every interrupt, capped at 16, and the ring was replayed forever: the key
/// start request ran again every 20 interrupts and reset the voice (OQ41).
pub const INDEX_BIAS: u32 = 0x8_0000;
/// VERIFIED (listing `0x1C05C9`: `r1 = dm(0x92419)`, `r0 = min(r0, r1)`; value 16 in the replay): the
/// SMP audio ISR takes at most `DM(0x92419)` pairs per interrupt. The SY1SY2 word `0x92180` is not
/// the cap of the SMP image (the model read it there, D185).
pub const FIFO_CAP: u32 = 0x9_2419;
/// Loader entry of the SMP image after the boot code.
pub const ENTRY: u32 = 0x1C_11E0;
/// Start-up status register the image polls (IOP space, written as plain memory).
pub const STATUS_ADDR: u32 = 0x5201;
pub const STATUS_VALUE: u32 = 0x40;
/// Pointer word the interrupt toggles between the two ring halves.
pub const RING_POINTER: u32 = 0xB_2028;
/// The two ring halves (20 words = 2 frames of a 10 word TDM frame each).
pub const RING_HALVES: [u32; 2] = [0xB_2000, 0xB_2014];
/// Words per TDM frame and frames per interrupt.
pub const TDM_WORDS: u32 = 10;
/// EMPIRICAL (run, Astoria Grand 3,000 key ticks, tap of both slots and both halves: the second
/// slot of the half (`+10`) and the other half read 0.0 in every interrupt, 0 of 5,968 words nonzero;
/// D200): an interrupt writes one frame, at `+0` of the half the pointer holds. The tap kept two
/// frames per interrupt until D200, so every other tap frame was a zero and the apparent pitch was
/// half the real one.
pub const FRAMES_PER_ISR: u32 = 1;

/// ASSUMED (the interrupt writes through the pointer it holds on entry and toggles at its end,
/// listing `0x1C0771`, `0x1C0786`): the ring half the next interrupt will write.
pub fn ring_half(core: &mut Core) -> u32 {
    let p = core.dm_read(RING_POINTER).unwrap_or(0);
    if RING_HALVES.contains(&p) {
        p
    } else {
        RING_HALVES[0]
    }
}

/// The other ring half than `half`.
pub fn other_half(half: u32) -> u32 {
    if half == RING_HALVES[0] {
        RING_HALVES[1]
    } else {
        RING_HALVES[0]
    }
}

/// The stereo frames `[left, right]` (float bits) an interrupt wrote into ring half `base`.
pub fn ring_frames(core: &mut Core, base: u32) -> Vec<[u32; 2]> {
    (0..FRAMES_PER_ISR)
        .map(|f| {
            let at = base + f * TDM_WORDS;
            [
                core.dm_read(at).unwrap_or(0),
                core.dm_read(at + 1).unwrap_or(0),
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core() -> Core {
        Core::new(&vec![0u8; 0x24_0000])
    }

    #[test]
    fn ring_frames_read_left_right_of_the_written_frame() {
        let mut c = core();
        c.dm_write(0xB_2000, 1).unwrap();
        c.dm_write(0xB_2001, 2).unwrap();
        c.dm_write(0xB_200A, 3).unwrap();
        c.dm_write(0xB_200B, 4).unwrap();
        // one frame per interrupt: the second TDM slot (`+10`) is not tapped (D200)
        assert_eq!(ring_frames(&mut c, RING_HALVES[0]), vec![[1, 2]]);
        assert_eq!(ring_frames(&mut c, RING_HALVES[1]), vec![[0, 0]]);
    }

    #[test]
    fn ring_half_follows_the_pointer_and_defaults_to_the_first() {
        let mut c = core();
        assert_eq!(ring_half(&mut c), RING_HALVES[0], "pointer not set yet");
        c.dm_write(RING_POINTER, RING_HALVES[1]).unwrap();
        assert_eq!(ring_half(&mut c), RING_HALVES[1]);
        c.dm_write(RING_POINTER, 0x1234).unwrap();
        assert_eq!(
            ring_half(&mut c),
            RING_HALVES[0],
            "garbage pointer is ignored"
        );
    }

    #[test]
    fn other_half_swaps_the_two_halves() {
        assert_eq!(other_half(RING_HALVES[0]), RING_HALVES[1]);
        assert_eq!(other_half(RING_HALVES[1]), RING_HALVES[0]);
    }

    #[test]
    fn constants_match_the_smp_scenarios() {
        assert_eq!((FIFO_BASE, ENTRY), (0x92196, 0x1C11E0));
        assert_eq!((STATUS_ADDR, STATUS_VALUE), (0x5201, 0x40));
    }
}
