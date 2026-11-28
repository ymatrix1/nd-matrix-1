//! Tests of the fast and slow access paths of [`super::Mem`]: the SRAM window against
//! `Core::dm_read` / `dm_write`, what the fast path declines, and the self-modified-code flag.
//!
//! OWNER: W2-MOVE, test build only.
use super::*;
use crate::core::sdram::Sdram;
use crate::core::support::core_with;
use crate::core::uop::verify::compare::exact_diff;
use crate::core::IOP_WORDS;

/// A core with every peripheral model and the census on.
fn all_models() -> Core {
    let mut c = core_with(&[0x0001]);
    c.enable_sc57x(0);
    c.enable_epdma();
    c.enable_epchain();
    c.enable_ringdma();
    c.enable_access_log();
    c.enable_spi_boot(vec![1, 2, 3]);
    c
}

/// The premise of the fast path: no peripheral model looks at an SRAM address, with every
/// model on, and no other window of `dm_read` / `dm_write` is in the way (the IOP array is
/// below, the SDRAM window above, the device store has no window).
#[test]
fn periph_claims_disjoint_from_sram_windows() {
    let c = all_models();
    // the census and the models are really on: the same function claims the IOP window and
    // the system range (without this the loop below would pass for any function)
    assert!(c.periph_claims(0x1_0000), "the IOP window under the census");
    assert!(c.periph_claims(0x2000_0000) && c.periph_claims(0x31FF_FFFF));
    assert!(c.periph_claims(0x1080) && c.periph_claims(0x1084));
    assert!(!c.periph_claims(0x3200_0000));
    let bare = core_with(&[0x0001]);
    assert!(!bare.periph_claims(0x1_0000), "no census, no claim");
    assert!(bare.periph_claims(0x2000_0000), "the range needs no model");
    // every address of both SRAM windows
    let start = SRAM_WORDS.start;
    let end = SHORT_WORDS.end;
    let claimed = (start..end).filter(|&a| c.periph_claims(a)).count();
    assert_eq!(claimed, 0, "claims inside {start:#x}..{end:#x}");
    // and just outside them nothing changes either way
    assert!(!c.periph_claims(start - 1) && !c.periph_claims(end));
    // the other windows of the memory model keep clear of the SRAM windows
    assert!(IOP_WORDS as u32 <= SRAM_WORDS.start);
    assert!(!Sdram::contains(SRAM_WORDS.start) && !Sdram::contains(SHORT_WORDS.end - 1));
    assert!(!Sdram::contains(IOP_WORDS as u32 - 1));
}

/// Fast reads and writes equal `dm_read` / `dm_write` on the SRAM window, including the
/// side effects of a write (the decode cache of the code word it hits).
#[test]
fn fast_access_equals_the_interpreters_memory_path() {
    let (mut fast, mut slow) = (core_with(&[0x0001; 4]), core_with(&[0x0001; 4]));
    for (k, a) in [0x9_8000u32, 0x9_8001, 0xA_1234, 0xF_FFFE, 0xF_FFFF]
        .into_iter()
        .enumerate()
    {
        let v = 0x1234_5678u32.rotate_left(k as u32 * 7);
        assert!(wr_fast(&mut fast, a, v));
        slow.dm_write(a, v).unwrap();
        assert_eq!(rd_fast(&fast, a), Some(v));
        assert_eq!(slow.dm_read(a), Ok(v));
    }
    assert_eq!(exact_diff(&fast, &slow), None);
    // a write into cached code: both cores run the NEW instruction afterwards. The four NOPs
    // are cached by running them; the word 0x9_0001 holds the parcels at 0x12_0002 and 0x12_0003
    for c in [&mut fast, &mut slow] {
        assert_eq!(c.run(4), None);
        (c.pc, c.r[0]) = (0x12_0002, 7);
    }
    // `R0 = DM(I2, M2)` (T3c, 16 bits) in the low parcel, a NOP in the high one
    let new_code = 0x0001_9490u32;
    assert!(wr_fast(&mut fast, 0x9_0001, new_code));
    slow.dm_write(0x9_0001, new_code).unwrap();
    for c in [&mut fast, &mut slow] {
        assert!(c.step().is_ok());
    }
    assert_eq!(fast.r[0], 0, "the stale NOP was not run");
    assert_eq!(exact_diff(&fast, &slow), None);
}

#[test]
fn the_fast_path_declines_what_the_interpreter_must_see() {
    let mut c = core_with(&[0x0001]);
    c.dm_write(0x9_0010, 5).unwrap();
    // a watched address, a window of them, and a read watch
    c.watch = 0x9_0010;
    assert!(!wr_fast(&mut c, 0x9_0010, 6) && wr_fast(&mut c, 0x9_0011, 6));
    assert_eq!(c.dm_peek(0x9_0010), Some(5), "nothing was written");
    c.watch_len = 4;
    assert!(!wr_fast(&mut c, 0x9_0013, 6) && wr_fast(&mut c, 0x9_0014, 6));
    assert!(rd_fast(&c, 0x9_0010).is_some());
    c.rwatch_on = true;
    assert!(rd_fast(&c, 0x9_0010).is_none(), "every read logs while on");
    c.rwatch_on = false;
    // outside the window: IOP, short-word alias, SDRAM, device, peripheral
    for a in [
        0x3_E018,
        0x8_FFFF,
        0x10_0000,
        0x12_4006,
        0x0802_0000,
        0x3108_D004,
    ] {
        assert!(rd_fast(&c, a).is_none(), "{a:#x}");
        assert!(!wr_fast(&mut c, a, 1), "{a:#x}");
    }
}

/// A write into a marked code page reaches `uop_note_write` through the fast path exactly as
/// it does through `dm_write`, and a write elsewhere does not.
#[test]
fn a_fast_write_into_translated_code_raises_the_smc_flag() {
    let (mut fast, mut slow) = (core_with(&[0x0001; 2]), core_with(&[0x0001; 2]));
    for c in [&mut fast, &mut slow] {
        c.uop_run.active = true;
        c.uop_run.code_pages.set_range(0x12_0000, 0x12_0100);
    }
    // a data word far from the marked page: no flag
    assert!(wr_fast(&mut fast, 0xA_0000, 7) && fast.uop_run.flags & RF_SMC == 0);
    assert!(!wr_plain(&fast, 0x9_0000), "its page is marked");
    assert!(wr_plain(&fast, 0xA_0000));
    // the first code word: flag, dirty bit and counter as `dm_write` leaves them
    assert!(wr_fast(&mut fast, 0x9_0000, 7));
    slow.dm_write(0x9_0000, 7).unwrap();
    assert_eq!(fast.uop_run.flags & RF_SMC, RF_SMC);
    assert_eq!(
        (
            fast.uop_run.flags,
            fast.uop_run.dirty,
            fast.uop_run.stats.smc
        ),
        (
            slow.uop_run.flags,
            slow.uop_run.dirty,
            slow.uop_run.stats.smc
        )
    );
    // the second page boundary: words 0x9_007F/0x9_0080 are short words 0x12_00FE..0x12_0101
    for (a, hit) in [(0x9_007Fu32, true), (0x9_0080, false)] {
        let mut c = core_with(&[0x0001]);
        c.uop_run.active = true;
        c.uop_run.code_pages.set_range(0x12_0000, 0x12_0100);
        assert!(wr_fast(&mut c, a, 1));
        assert_eq!(c.uop_run.flags & RF_SMC != 0, hit, "{a:#x}");
    }
}
