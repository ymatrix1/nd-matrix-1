//! The circular-buffer latch of a move uop that leaves its block early (`Mem::leave`).
//!
//! OWNER: W2-MOVE. The dispatch loop latches a wrap of `I7` / `I15` (`CB7S` / `CB15S`) only after
//! an `Ok` exit; a `UF_CB` uop that wraps and then ends with `Smc` (a store into translated
//! code) or `Jump` (a slow access moved `irq.due`) would leave `cb_wrap` set until the next
//! instruction, and the interpreter latches it at the end of its own step. `Mem::leave` therefore
//! latches before it returns either exit. Each case runs the uop through the handler protocol of
//! `support::dispatch` (which, like the real loop, skips the latch on those two exits) and
//! compares with `Core::step` through `exact_diff`, so a handler that forgot the latch differs in
//! `STKY` and `cb_wrap`. The no-wrap twin of every case shows that the wrap is what is under test.
use super::support::{run, Run};
use super::tests_xfer::{mark_code, t4b_post, CBUFEN};
use crate::core::periph::spi_boot::{CSPI, IISPI, IMSPI, SPIDMAC, SPIHI_VECTOR};
use crate::core::uop::run::RF_SMC;
use crate::core::uop::types::{Exit, UF_CB};
use crate::core::uop::verify::gen::CODE_PC;
use crate::core::Core;

/// The write bit of a type 4 instruction (bit 39 of the instruction word, bit 7 of the first
/// parcel): `DM(Ii, 2) = R1` instead of `R1 = DM(Ii, 2)`.
const WRITE: u16 = 0x80;
/// The normal-word address of the first instruction (`CODE_PC` counts short words).
const CODE_WORD: u32 = CODE_PC / 2;

/// `L` / `B` / `I` of `reg` for an access at `at`: with `wraps` a buffer of two words whose second
/// word is `at`, so the post-modify by 2 leaves it; without, a buffer of eight words from `at`.
fn buffer(c: &mut Core, reg: usize, at: u32, wraps: bool) {
    let (b, l) = if wraps { (at - 1, 2) } else { (at, 8) };
    (c.b[reg], c.l[reg]) = (b, l);
    c.i[reg] = at;
    c.mode1 |= CBUFEN;
}

/// `DM(I7, 2) = R1` (`DM(I15, 2)` with `bank`): a store at `I` into a marked page of translated
/// code, which ends the uop with `Smc`.
fn store_into_code(bank: u16, wraps: bool, at: u32) -> Run {
    let reg = 7 + usize::from(bank) * 8;
    let mut p = t4b_post(7, bank);
    p[0] |= WRITE;
    let tweak = move |c: &mut Core| {
        mark_code(c);
        buffer(c, reg, at, wraps);
    };
    run(&p, 111 + u64::from(bank), false, false, &tweak)
}

/// `R1 = DM(I7, 2)` (`DM(I15, 2)`) from `SPIDMAC` while a boot DMA waits: the poll completes the
/// transfer and raises `SPIHI`, which moves `irq.due` to 0 and ends the uop with `Jump`.
fn poll_that_raises_an_interrupt(bank: u16, wraps: bool) -> Run {
    let reg = 7 + usize::from(bank) * 8;
    let tweak = move |c: &mut Core| {
        c.enable_spi_boot(vec![7, 8, 9]);
        c.dm_write(IISPI, 0x9_2003).unwrap();
        c.dm_write(IMSPI, 1).unwrap();
        c.dm_write(CSPI, 3).unwrap();
        buffer(c, reg, SPIDMAC, wraps);
        c.irq.due = u64::MAX;
    };
    run(
        &t4b_post(7, bank),
        121 + u64::from(bank),
        false,
        false,
        &tweak,
    )
}

/// The STKY bit the wrap of `I7` (17) or `I15` (18) sets.
fn sticky(r: &Run, bank: u16) -> bool {
    (r.got.sys.stky[0] >> (17 + u32::from(bank))) & 1 == 1
}

#[test]
fn a_wrap_is_latched_when_the_uop_ends_with_smc() {
    let mut latched = 0;
    for bank in [0u16, 1] {
        for wraps in [true, false] {
            let at = CODE_WORD + 0x11;
            let r = store_into_code(bank, wraps, at);
            let what = format!("I{} wraps {wraps}", 7 + bank * 8);
            assert_eq!(r.diff, None, "{what}");
            assert!(r.taken && r.uops.len() == 1, "{what}");
            assert!(r.uops[0].flags & UF_CB != 0, "UF_CB on {what}");
            assert_eq!(r.exit, Some(Exit::Smc), "{what}");
            assert!(
                r.got.uop_run.flags & RF_SMC != 0,
                "the store hit code: {what}"
            );
            assert_eq!(r.got.dm_peek(at), Some(r.got.r[1]), "the store ran: {what}");
            assert_eq!(sticky(&r, bank), wraps, "STKY of {what}");
            assert_eq!(r.got.cb_wrap.get(), 0, "latched by the handler: {what}");
            latched += u32::from(wraps && sticky(&r, bank));
        }
    }
    assert_eq!(latched, 2, "both index registers wrapped and were latched");
}

#[test]
fn a_wrap_is_latched_when_the_uop_ends_with_jump() {
    let mut latched = 0;
    for bank in [0u16, 1] {
        for wraps in [true, false] {
            let r = poll_that_raises_an_interrupt(bank, wraps);
            let what = format!("I{} wraps {wraps}", 7 + bank * 8);
            assert_eq!(r.diff, None, "{what}");
            assert!(r.taken && r.uops.len() == 1, "{what}");
            assert!(r.uops[0].flags & UF_CB != 0, "UF_CB on {what}");
            assert_eq!(r.exit, Some(Exit::Jump), "{what}");
            assert_ne!(
                r.got.irq.irptl & 1 << SPIHI_VECTOR,
                0,
                "SPIHI raised: {what}"
            );
            assert_eq!(r.got.irq.due, 0, "the event is due now: {what}");
            assert_eq!(
                r.got.pc,
                CODE_PC + 2,
                "the pc after the 2-parcel insn: {what}"
            );
            assert_eq!(sticky(&r, bank), wraps, "STKY of {what}");
            assert_eq!(r.got.cb_wrap.get(), 0, "latched by the handler: {what}");
            latched += u32::from(wraps && sticky(&r, bank));
        }
    }
    assert_eq!(latched, 2, "both index registers wrapped and were latched");
}
