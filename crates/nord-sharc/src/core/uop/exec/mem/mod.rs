//! Memory access paths of the move handlers.
//!
//! OWNER: W2-MOVE. The fast path is the SRAM normal-word window (`memory.rs::SRAM_WORDS`): a read
//! or a write goes straight to `Core::sram`, exactly as `Core::dm_read` / `Core::dm_write` do for
//! an address in that window, and everything else (IOP, device store, SDRAM, the short-word alias
//! window, a watched or read-watched address) goes through `dm_read` / `dm_write` unchanged, so the
//! order of accesses, the watch logs and the peripheral models see exactly what the interpreter
//! shows them.
//!
//! Why the shortcut is exact (each point has a test below):
//!
//! * `dm_read` and `dm_write` look at the peripheral models first, but `periph_claims` is false
//!   for every SRAM address, with every model on (`periph_claims_disjoint_from_sram_windows`), and
//!   the IOP array and the SDRAM window lie below and above the SRAM window.
//! * The watch logs read `core.pc` and `core.steps`; the block only keeps them current at a uop
//!   boundary, so the slow path materializes both first (`Mem::enter_slow`), and a watched write
//!   or any read while `rwatch_on` takes the slow path.
//! * A write into the SRAM window must tell the decode cache, the JIT and the uop engine about
//!   self-modified code. The decode cache only holds instructions inside `sram_code_nw_lo..=hi`
//!   (`fetch_and_cache` widens that extent for every entry it inserts and `invalidate_code_cache`
//!   resets both together with the cache, the precedent of v1 `aot_wr`), the uop engine marks the
//!   pages of translated code in `uop_run.code_pages`, and the JIT is asked whenever it exists.
//!   A write inside none of the three calls nothing, which is what `invalidate_sram_cache` would
//!   have done to a cache that holds nothing there.
//!
//! [`Mem`] bundles what one handler needs: the core, the context, the uop and the "due guard" of
//! rule 4 of design 4.5. The guard is a single `u64`: the value of `irq.due` before the first slow
//! access of the uop (0 while there was none, which no later value can be below), so the exit
//! check after the accesses is one compare. A handler ends with [`Mem::finish`], which returns
//! `Ok`, or `Smc` when a write hit translated code, or `Jump` to the next instruction when an
//! access moved the event deadline (the uop completed; the driver re-enters with a fresh room).
//!
//! Operand convention shared by every move uop: bits 30..31 of operand `a` hold the length of the
//! instruction in short words ([`LEN_SHIFT`]), which is how a `Jump` finds the pc after it.
use super::super::run::{CODE_BASE, CODE_PAGE_WORDS, RF_SMC};
use super::super::types::{Exit, Uop, UopCtx, PAGE_SHIFT, UF_CB, UF_FIN};
use crate::core::memory::{SHORT_WORDS, SRAM_WORDS};
use crate::core::Core;

/// Bit position of the instruction length (1 to 3 short words) in operand `a` of a move uop.
/// VERIFIED: the longest instruction is 48 bits, three short words (`visa::classify`); checked by
/// `insn_length_round_trips`.
pub(crate) const LEN_SHIFT: u32 = 30;

/// The access did not complete (`dm_read` / `dm_write` returned an error). VERIFIED: neither
/// function returns an error today (`memory.rs` ends every path in `Ok`), so the handlers answer
/// `Exit::Fault` for the first access of a uop, before any state changed; the arm is kept so a
/// future faulting model cannot be silently ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Bad;

/// Whether normal-word data address `addr` is backed by the internal SRAM.
#[inline(always)]
pub(crate) fn in_sram_window(addr: u32) -> bool {
    SRAM_WORDS.contains(&addr)
}

/// Whether `addr` is in the short-word alias window of the SRAM (writes there reach SRAM through
/// `dm_write`, which calls the invalidation hook, so handlers never bypass it).
#[inline(always)]
pub(crate) fn in_short_window(addr: u32) -> bool {
    SHORT_WORDS.contains(&addr)
}

/// The word at `addr` when it can be read without the interpreter's memory path: an SRAM address
/// while no read watch is armed.
#[inline(always)]
pub(crate) fn rd_fast(core: &Core, addr: u32) -> Option<u32> {
    if core.rwatch_on || !in_sram_window(addr) {
        return None;
    }
    let o = addr as usize * 4;
    let b = core.sram.get(o..o + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Whether the page of translated code that holds short word `first_short` is marked.
#[inline(always)]
fn page_marked(core: &Core, first_short: u32) -> bool {
    let page = (first_short.wrapping_sub(CODE_BASE) >> PAGE_SHIFT) as usize;
    core.uop_run.code_pages.bits[(page >> 6) & (CODE_PAGE_WORDS - 1)] >> (page & 63) & 1 != 0
}

/// Whether a write at `addr` must be reported to the code caches.
#[inline(always)]
fn code_cached(core: &Core, addr: u32) -> bool {
    #[cfg(feature = "jit")]
    if core.jit.is_some() {
        return true;
    }
    (addr >= core.sram_code_nw_lo && addr <= core.sram_code_nw_hi)
        || page_marked(core, addr.wrapping_mul(2))
}

/// Whether [`wr_fast`] can store at `addr` and cannot reach translated code: an SRAM address
/// that is not watched and whose code page is not marked. A uop that must not return `Smc`
/// (it carries `UF_FIN`) checks this before it mutates anything.
#[inline(always)]
pub(crate) fn wr_plain(core: &Core, addr: u32) -> bool {
    in_sram_window(addr)
        && addr.wrapping_sub(core.watch) >= core.watch_len
        && !page_marked(core, addr.wrapping_mul(2))
}

/// A store reached cached code: tells the decode cache, the JIT and the uop engine
/// (`invalidate_sram_cache`, which also sets `RF_SMC`). Out of line: it is the rare path of a
/// store handler, and the invalidation inlined there would triple its size.
#[cold]
#[inline(never)]
fn note_code_write(core: &mut Core, addr: u32) {
    core.invalidate_sram_cache(addr.wrapping_mul(2), 2);
}

/// Stores `v` at `addr` when that needs no part of the interpreter's memory path: an SRAM address
/// that is not watched. Returns whether it stored. A store that hits cached code reports it to the
/// caches ([`note_code_write`]).
#[inline(always)]
pub(crate) fn wr_fast(core: &mut Core, addr: u32, v: u32) -> bool {
    if !in_sram_window(addr) || addr.wrapping_sub(core.watch) < core.watch_len {
        return false;
    }
    let o = addr as usize * 4;
    let Some(b) = core.sram.get_mut(o..o + 4) else {
        return false;
    };
    b.copy_from_slice(&v.to_le_bytes());
    if code_cached(core, addr) {
        note_code_write(core, addr);
    }
    true
}

/// The memory side of one uop: reads and writes with the interpreter's semantics, and the exit
/// the uop owes the dispatch loop afterwards.
pub(crate) struct Mem<'a> {
    pub core: &'a mut Core,
    ctx: &'a mut UopCtx,
    pub u: &'a Uop,
    /// `irq.due` before the first slow access of this uop, 0 while there was none.
    chk: u64,
}

impl<'a> Mem<'a> {
    /// The accessor of uop `u` running in `ctx`.
    ///
    /// # Safety
    /// `ctx` and `u` must be valid for the whole call and `ctx.core` must point to the live core
    /// of the block (the handler ABI guarantees both).
    #[inline(always)]
    pub(crate) unsafe fn new(ctx: *mut UopCtx, u: *const Uop) -> Mem<'a> {
        let ctx = &mut *ctx;
        let core = &mut *ctx.core;
        Mem {
            core,
            ctx,
            u: &*u,
            chk: 0,
        }
    }

    /// Whether the uop must run `finish_step` after it (a delay slot or a loop end). Such a uop
    /// cannot return `Jump` or `Smc` (the dispatch loop would skip `finish_step`), so it checks
    /// [`Mem::can_read`] and [`Mem::can_write`] first and returns `Slow` when an access would
    /// not be plain.
    #[inline(always)]
    pub(crate) fn fin(&self) -> bool {
        self.u.flags & UF_FIN != 0
    }

    /// A read of `addr` takes the fast path.
    #[inline(always)]
    pub(crate) fn can_read(&self, addr: u32) -> bool {
        !self.core.rwatch_on && in_sram_window(addr)
    }

    /// A write at `addr` takes the fast path and cannot hit translated code.
    #[inline(always)]
    pub(crate) fn can_write(&self, addr: u32) -> bool {
        wr_plain(self.core, addr)
    }

    /// Materializes the core at the uop boundary and arms the due guard: the watch logs, the
    /// peripheral census and the interrupt controller read `pc` and `steps`.
    #[cold]
    #[inline(never)]
    fn enter_slow(&mut self) {
        self.core.pc = self.u.pc;
        self.core.steps = self.ctx.base_steps + u64::from(self.ctx.done);
        if self.chk == 0 {
            self.chk = self.core.irq.due;
        }
    }

    #[cold]
    #[inline(never)]
    fn rd_slow(&mut self, addr: u32) -> Result<u32, Bad> {
        self.enter_slow();
        self.core.dm_read(addr).map_err(|_| Bad)
    }

    #[cold]
    #[inline(never)]
    fn wr_slow(&mut self, addr: u32, v: u32) -> Result<(), Bad> {
        self.enter_slow();
        self.core.dm_write(addr, v).map_err(|_| Bad)
    }

    /// `Core::dm_read` of `addr`.
    #[inline(always)]
    pub(crate) fn rd(&mut self, addr: u32) -> Result<u32, Bad> {
        match rd_fast(self.core, addr) {
            Some(v) => Ok(v),
            None => self.rd_slow(addr),
        }
    }

    /// `Core::dm_write` of `v` at `addr`.
    #[inline(always)]
    pub(crate) fn wr(&mut self, addr: u32, v: u32) -> Result<(), Bad> {
        if wr_fast(self.core, addr, v) {
            Ok(())
        } else {
            self.wr_slow(addr, v)
        }
    }

    /// The exit code of the uop after its accesses: `Ok`, or the hand-over of
    /// [`Mem::leave`]. `store` says the uop wrote memory (a write can hit translated code even
    /// through the fast path; after a read only a slow access can have side effects).
    #[inline(always)]
    pub(crate) fn finish(&mut self, store: bool) -> u32 {
        if (store && self.core.uop_run.flags & RF_SMC != 0) || self.chk != 0 {
            return self.leave();
        }
        Exit::Ok as u32
    }

    /// A store reached translated code, or a slow access ran: report it. `Smc`: the uop
    /// completed and the rest of the block may be stale. `Jump` to the pc after the instruction:
    /// the uop completed and an access moved `irq.due` below its value before the uop, so the
    /// room the block was entered with is no longer true. The circular-buffer latch the dispatch
    /// loop owes a `UF_CB` uop runs here first, because neither exit passes through it (pinned by
    /// `h_move::tests_cb`: without it `CB7S` / `CB15S` would be latched one instruction late).
    #[cold]
    #[inline(never)]
    fn leave(&mut self) -> u32 {
        if self.u.flags & UF_CB != 0 && self.core.cb_wrap.get() != 0 {
            self.core.latch_cb_overflow();
        }
        if self.core.uop_run.flags & RF_SMC != 0 {
            return Exit::Smc as u32;
        }
        if self.core.irq.due < self.chk {
            self.ctx.next_pc = self.u.pc.wrapping_add(self.u.a >> LEN_SHIFT);
            return Exit::Jump as u32;
        }
        Exit::Ok as u32
    }
}

#[cfg(test)]
mod tests_access;
#[cfg(test)]
mod tests_exit;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_windows_are_the_ones_of_memory_rs_and_do_not_overlap() {
        assert!(in_sram_window(0x9_0000) && in_sram_window(0xF_FFFF) && !in_sram_window(0x10_0000));
        assert!(
            in_short_window(0x10_0000) && in_short_window(0x1F_FFFF) && !in_short_window(0x20_0000)
        );
        assert!(!in_sram_window(0x8_FFFF) && !in_short_window(0xF_FFFF));
        assert!(
            !in_sram_window(0x2000_0000),
            "the peripheral range is outside both windows"
        );
        const _: () = assert!(SRAM_WORDS.end <= SHORT_WORDS.start);
    }

    #[test]
    fn insn_length_round_trips() {
        for len in 1..=3u32 {
            let a = 0x12_3456 | len << LEN_SHIFT;
            assert_eq!(a >> LEN_SHIFT, len);
        }
        assert_eq!(3u32 << LEN_SHIFT, 0xC000_0000);
    }
}
