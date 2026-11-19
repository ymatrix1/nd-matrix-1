//! Small state types of the core: the fault that stops a run, the condition flags of one
//! processing element, the loop stack entry and the pending delayed branch.

/// Why a run stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// The instruction at `pc` is not modelled yet (`parcels` holds its raw words).
    Unimplemented { pc: u32, parcels: [u16; 3] },
    /// A data or instruction access outside the modelled memory.
    BadAddress { pc: u32, address: u32 },
}

/// Condition flags used by `IF cond` (ASTATx subset plus loop counter expiry).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Flags {
    pub az: bool,
    pub an: bool,
    pub av: bool,
    pub ac: bool,
    /// Float invalid-operation flag (AI) and the sign flag of ABS/MANT (AS).
    pub ai: bool,
    pub as_: bool,
    pub tf: bool,
    /// Shifter flags: zero result, overflow and sign.
    pub sz: bool,
    pub sv: bool,
    pub ss: bool,
    /// Shifter bit FIFO of this processing element (`SF` is derived from its write pointer).
    pub fifo: super::bitfifo::BitFifo,
}

/// Loop stack entry of `DO ... UNTIL cond` (`arith_loop`) and `DO ... UNTIL LCE`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LoopFrame {
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) remaining: u32,
    /// Termination condition code of an arithmetic loop (Type 13a); `None` for a counter loop.
    pub(crate) term: Option<u8>,
    /// Address whose execution evaluates the termination condition (`arith_loop`); `u32::MAX` =
    /// never.
    pub(crate) check: u32,
    /// Instructions an arithmetic loop still runs after its condition held (0 = not leaving).
    pub(crate) exit_in: u8,
    /// The instruction after the loop is skipped (a one instruction loop ended by the sample
    /// of the `DO` itself).
    pub(crate) skip_after: bool,
    /// Return addresses below this loop's top-of-loop entry on the shared PC stack (`pcstack`).
    pub(crate) cs_depth: usize,
    /// The top-of-loop address is not on the PC stack (`POP PCSTK` or a fresh `PUSH LOOP`).
    pub(crate) detached: bool,
}

/// A delayed branch waiting for its two delay-slot instructions to finish.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pending {
    /// Steps left including the one that issued the branch (3 = just issued).
    pub(crate) slots: u8,
    pub(crate) target: u32,
    pub(crate) call: bool,
}

#[inline]
pub(super) fn sign_extend(value: u32, width: u32) -> u32 {
    let shift = 32 - width;
    (((value << shift) as i32) >> shift) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_extension_keeps_the_low_bits_and_copies_the_top_one() {
        assert_eq!(sign_extend(0xFFFF, 16), 0xFFFF_FFFF);
        assert_eq!(sign_extend(0x7FFF, 16), 0x7FFF);
        assert_eq!(sign_extend(0x80, 8), 0xFFFF_FF80);
    }

    #[test]
    fn flags_default_to_all_clear() {
        let f = Flags::default();
        assert!(!(f.az || f.an || f.av || f.ac || f.ai || f.as_ || f.tf || f.sz || f.sv || f.ss));
    }
}
