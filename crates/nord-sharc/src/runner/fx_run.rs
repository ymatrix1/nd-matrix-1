//! FX chip Core2 run with live audio (D122): the FX image pass is started by the mailbox, the
//! Core1 hand-over counter is played by [`FxPeer`], and 4 stereo frames per pass go in through
//! the pass input block and come out through the output pointer `DM(0xC03D7)`.
//!
//! Everything here is opt-in (`sharc-fxlive`); `sharc-sys` and the other runners do not use it.
//! Provenance (`docs/research/fx-oracle-live.md`): the addresses are read from the Core2 listing
//! (`VERIFIED`), the hand-over counter protocol and the fixed ping-pong choice are `ASSUMED`.
pub use super::fx_probe::{calls_at, regs_at};
pub use super::fx_wav::read_wav;
use super::msg::{deliver, Msg};
use super::pass::{fault_text, run_steps, PassConfig};
use super::periph_run::PeriphOpts;
use super::system::lean_pass;
use crate::core::Core;

/// Output location of a pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutLoc {
    /// The pass stores 8 words behind the pointer held in this data word (Core2).
    Pointer(u32),
    /// The master words sit at this address (Core1, `DM(0xC04D8..)`).
    Fixed(u32),
}

/// One core of the FX chip: how it is booted, started, fed and read (VERIFIED from the listings,
/// `docs/research/fx-oracle-live.md` s2; the choice of one fixed ping buffer is `ASSUMED`).
#[derive(Clone, Copy, Debug)]
pub struct FxCore {
    pub name: &'static str,
    pub entry: u32,
    pub idle_pc: u32,
    pub trigger: u32,
    /// Receive interrupt `(vector, SEC source)` that starts a pass (Core1), else the mailbox.
    pub irq: Option<(u8, u32)>,
    /// Device words the boot needs (core flags, board gate, DMA status).
    pub devs: &'static [(u32, u32)],
    /// FLAG1 input high (Core1 waits on it at start-up).
    pub flag1: bool,
    /// Word compared with [`PING_VALUE`] to choose the ping buffer.
    pub ping_select: u32,
    /// Ping input block (Core1: the PNO 2D block, Core2: the layer block).
    pub input_base: u32,
    pub out: OutLoc,
    /// Sign of the output words (the Core1 master stage stores the negated sum).
    pub out_sign: f32,
    /// Plays the Core1 hand-over counter (Core2 only).
    pub peer: bool,
}

/// FX Core1 (`DM(0xC02C1)` mailbox, P4I vector 15 with SEC source `0x41`).
pub const FX1: FxCore = FxCore {
    name: "FX core 1",
    entry: 0x12_11E3,
    idle_pc: 0x12_128B,
    trigger: 0xC_02C1,
    irq: Some((15, 0x41)),
    devs: &[
        (0x3108_C06C, 0x400),
        (0x3108_C00C, 4),
        (0x28B0_0B00, 1),
        (0x3108_D008, 5),
    ],
    flag1: true,
    ping_select: 0xC_0450,
    input_base: 0xC_01BC,
    out: OutLoc::Fixed(0xC_04D8),
    out_sign: -1.0,
    peer: false,
};

/// FX Core2 (mailbox start, waits for the Core1 counter).
pub const FX2: FxCore = FxCore {
    name: "FX core 2",
    entry: 0x12_11E3,
    idle_pc: 0x12_125B,
    trigger: 0xC_02C2,
    irq: None,
    devs: &[(0x2830_0B00, 1), (0x3108_D008, 5)],
    flag1: false,
    ping_select: 0xC_03D3,
    input_base: 0xC_0180,
    out: OutLoc::Pointer(0xC_03D7),
    out_sign: 1.0,
    peer: true,
};

/// Word the ping-select cell is set to (VERIFIED, `0x12126B..` Core2 listing: equal selects the
/// ping block).
pub const PING_VALUE: u32 = 0x2830_0830;
/// The pass counter of Core1 (alias of its `DM(0xB0141)`) and the copy Core2 keeps.
pub const PEER_COUNTER: u32 = 0x282C_0504;
pub const SELF_COUNTER: u32 = 0xB_0141;
/// Stereo frames per FX pass.
pub const FRAMES_PER_PASS: usize = 4;

impl FxCore {
    /// The pass configuration (mailbox or interrupt start).
    pub fn config(&self) -> PassConfig {
        PassConfig {
            trigger: self.trigger,
            idle_pc: self.idle_pc,
            max_steps: 400_000,
            irq: self.irq,
        }
    }

    /// Boots a core from its flat SRAM image: gates preset, FLAG1, `boot` instructions.
    pub fn boot(&self, data: &[u8], boot: u64, periph: &PeriphOpts) -> Result<Core, String> {
        let mut core = Core::new(data);
        periph.attach(&mut core);
        core.pc = self.entry;
        if self.flag1 {
            core.set_flag_input(1, true);
        }
        for &(a, v) in self.devs {
            core.preset_device(a, v);
        }
        match run_steps(&mut core, boot) {
            Some(f) => Err(format!("boot faulted: {}", fault_text(&f))),
            None => Ok(core),
        }
    }

    /// Reads the 4 stereo frames the pass produced (sign applied).
    pub fn read_output(&self, core: &mut Core) -> ([f32; 4], [f32; 4]) {
        let base = match self.out {
            OutLoc::Pointer(p) => core.dm_read(p).unwrap_or(0),
            OutLoc::Fixed(a) => a,
        };
        read_frames(core, base, self.out_sign)
    }
}

/// Reads 4 stereo frames `[L0 R0 L1 R1 ..]` at `base`, each word times `sign`.
pub fn read_frames(core: &mut Core, base: u32, sign: f32) -> ([f32; 4], [f32; 4]) {
    {
        let (mut l, mut r) = ([0f32; 4], [0f32; 4]);
        for k in 0..4u32 {
            let mut w =
                |o: u32| sign * f32::from_bits(core.dm_read(base.wrapping_add(o)).unwrap_or(0));
            l[k as usize] = w(2 * k);
            r[k as usize] = w(2 * k + 1);
        }
        (l, r)
    }
}

/// Plays the side of Core1 in the pass hand-over (the wait at `0x1212E4`: Core2 continues when
/// `DM(0x282C0504) & 7 == DM(0xB0141) - 1`). ASSUMED protocol: pass 0 presents counter 0 (the
/// pass that rebases the pointer triples), later passes cycle 1..7 so the rebase never repeats.
#[derive(Clone, Copy, Debug, Default)]
pub struct FxPeer {
    pass: u32,
}

impl FxPeer {
    /// Counter that Core1 shows for the next pass.
    pub fn counter(&self) -> u32 {
        if self.pass == 0 {
            0
        } else {
            (self.pass - 1) % 7 + 1
        }
    }

    /// Presents the counter and the ping buffer choice for the next pass.
    pub fn before_pass(&mut self, core: &mut Core) {
        let c = self.counter();
        core.preset_device(PEER_COUNTER, c);
        let _ = core.dm_write(SELF_COUNTER, c + 1);
        self.pass += 1;
    }
}

/// What an FX core needs from its environment before every pass: the Core1 hand-over counter
/// (Core2 only) and the ping-buffer selector. Without it the Core2 pass loses its poll loop after
/// pass 0 (`docs/research/pno-jit-arm-profile.md`: it waits at `0x1212E4` for the counter); one
/// value serves `fx_live_run`, `pass_sched` and `lle_sharc_bench`.
#[derive(Clone, Copy, Debug)]
pub struct FxDrive {
    pub core: FxCore,
    peer: FxPeer,
}

impl FxDrive {
    pub fn new(core: FxCore) -> Self {
        Self {
            core,
            peer: FxPeer::default(),
        }
    }

    /// Presents the peer counter (when the core has a peer) and the ping selector.
    pub fn before_pass(&mut self, core: &mut Core) {
        if self.core.peer {
            self.peer.before_pass(core);
        }
        let _ = core.dm_write(self.core.ping_select, PING_VALUE);
    }
}

/// Stereo output of a live FX run.
#[derive(Debug, Default, PartialEq)]
pub struct FxLive {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    /// Frames read from the tap block after every pass (empty without a tap).
    pub tap_left: Vec<f32>,
    pub tap_right: Vec<f32>,
    pub passes: u32,
    pub first_steps: u64,
    pub last_steps: u64,
}

/// Writes 4 stereo frames as `[L0 R0 L1 R1 ..]` at `base`.
pub fn write_block(core: &mut Core, base: u32, l: &[f32], r: &[f32]) {
    for (k, (a, b)) in l.iter().zip(r).take(FRAMES_PER_PASS).enumerate() {
        let _ = core.dm_write(base + 2 * k as u32, a.to_bits());
        let _ = core.dm_write(base + 2 * k as u32 + 1, b.to_bits());
    }
}

/// Runs `total` passes. Before pass `p` the due messages are delivered and `inject(p, core)`
/// writes the input block; after it the output block (and the block at `tap`, when given, for
/// example a bus the stages work on) is appended to the result. A pass that does
/// not return to the poll loop ends the run with an error.
pub fn fx_live_run(
    fx: &FxCore,
    core: &mut Core,
    msgs: &[Msg],
    total: u32,
    tap: Option<u32>,
    inject: &mut dyn FnMut(u32, &mut Core),
) -> Result<FxLive, String> {
    let mut due: Vec<&Msg> = msgs.iter().collect();
    due.sort_by_key(|m| m.pass);
    let cfg = fx.config();
    let (mut next, mut drive, mut out) = (0, FxDrive::new(*fx), FxLive::default());
    for p in 0..total {
        while next < due.len() && due[next].pass <= p {
            deliver(core, 0, due[next])?;
            next += 1;
        }
        drive.before_pass(core);
        inject(p, core);
        let (steps, idle) = lean_pass(core, &cfg);
        if !idle {
            return Err(format!(
                "pass {p} did not return to the poll loop ({steps} steps)"
            ));
        }
        let (l, r) = fx.read_output(core);
        out.left.extend(l);
        out.right.extend(r);
        if let Some(t) = tap {
            let (l, r) = read_frames(core, t, 1.0);
            out.tap_left.extend(l);
            out.tap_right.extend(r);
        }
        out.passes += 1;
        if p == 0 {
            out.first_steps = steps;
        }
        out.last_steps = steps;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::pass::tests::clearing_core;
    use super::*;

    #[test]
    fn the_peer_counter_rebases_once_and_then_cycles_one_to_seven() {
        let mut p = FxPeer::default();
        let mut c = clearing_core();
        let seen: Vec<u32> = (0..10)
            .map(|_| {
                let v = p.counter();
                p.before_pass(&mut c);
                v
            })
            .collect();
        assert_eq!(seen, [0, 1, 2, 3, 4, 5, 6, 7, 1, 2]);
        assert_eq!(c.dm_read(SELF_COUNTER), Ok(3), "own copy = counter + 1");
    }

    #[test]
    fn the_drive_presents_counter_and_ping_select_before_every_pass() {
        let mut c = clearing_core();
        let mut d = FxDrive::new(FX2);
        for _ in 0..3 {
            d.before_pass(&mut c);
        }
        assert_eq!(c.dm_read(SELF_COUNTER), Ok(3), "pass 2 shows counter 2");
        assert_eq!(c.dm_read(FX2.ping_select), Ok(PING_VALUE));
        // Core1 has no peer: its own counter cell is left alone, the selector is still set
        let mut c1 = clearing_core();
        let mut d1 = FxDrive::new(FX1);
        d1.before_pass(&mut c1);
        assert_eq!(c1.dm_read(SELF_COUNTER), Ok(0));
        assert_eq!(c1.dm_read(FX1.ping_select), Ok(PING_VALUE));
    }

    #[test]
    fn blocks_round_trip_through_data_memory() {
        let mut c = clearing_core();
        write_block(
            &mut c,
            0xB_0200,
            &[1.0, 2.0, 3.0, 4.0],
            &[-1.0, -2.0, -3.0, -4.0],
        );
        assert_eq!(c.dm_read(0xB_0202), Ok(2.0f32.to_bits()));
        assert_eq!(c.dm_read(0xB_0203), Ok((-2.0f32).to_bits()));
        c.dm_write(0xC_03D7, 0xB_0200).unwrap();
        let (l, r) = FX2.read_output(&mut c);
        assert_eq!((l, r), ([1.0, 2.0, 3.0, 4.0], [-1.0, -2.0, -3.0, -4.0]));
        // Core1 reads the master words in place and flips the sign of the negated sum
        let (l, r) = FX1.read_output(&mut c);
        assert_eq!((l, r), ([0.0; 4], [0.0; 4]));
        write_block(&mut c, 0xC_04D8, &[0.5; 4], &[0.25; 4]);
        assert_eq!(FX1.read_output(&mut c), ([-0.5; 4], [-0.25; 4]));
    }

    #[test]
    fn a_run_collects_four_frames_per_pass_and_reports_a_stuck_pass() {
        let mut c = clearing_core();
        let fx = FxCore {
            trigger: 0xB_00E2,
            idle_pc: 0x12_0002,
            out: OutLoc::Fixed(0xB_0300),
            out_sign: 1.0,
            peer: false,
            irq: None,
            ..FX2
        };
        let mut seen = Vec::new();
        let r = fx_live_run(&fx, &mut c, &[], 1, Some(0xB_0300), &mut |p, core| {
            seen.push(p);
            write_block(core, 0xB_0300, &[0.5; 4], &[0.25; 4]);
        })
        .unwrap();
        assert_eq!((r.passes, r.left.len(), seen.as_slice()), (1, 4, &[0][..]));
        assert_eq!((r.left[0], r.right[3]), (0.5, 0.25));
        assert_eq!(r.tap_left, r.left, "the tap block is read like the output");
        // the second pass hits the step limit of the tiny core
        let e = fx_live_run(&fx, &mut c, &[], 3, None, &mut |_, _| {}).unwrap_err();
        assert!(e.contains("did not return"), "{e}");
    }

    #[test]
    fn core_descriptors_boot_a_tiny_image_and_report_a_boot_fault() {
        let data = vec![0u8; 0x24_0000 + 64];
        assert_eq!(FX1.config().irq, Some((15, 0x41)));
        assert_eq!(FX2.config().trigger, 0xC_02C2);
        let c = FX2.boot(&data, 0, &PeriphOpts::default()).unwrap();
        assert_eq!(c.pc, FX2.entry);
        // an undecodable instruction word at the entry pc is a boot fault
        let at = 0x24_0000 + 2 * (FX1.entry as usize - 0x12_0000);
        let mut bad = vec![0u8; at + 64];
        bad[at..at + 6].fill(0xFF);
        assert!(FX1.boot(&bad, 5, &PeriphOpts::default()).is_err());
    }
}
