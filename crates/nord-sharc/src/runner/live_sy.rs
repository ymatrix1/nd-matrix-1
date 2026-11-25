//! The SY (sample) chip of `sharc-live` (D153): the interrupt-driven pass model.
//!
//! The ORG and PNO chips run one pass by a mailbox write (or one Core1 interrupt) and stop in a
//! poll loop. The SY1SY2 image has no poll loop: its work is the audio interrupt (vector 17), one
//! interrupt per output frame, and its command FIFO is re-armed by the FIFO reset interrupt
//! (vector 13) (`docs/research/sy-smp-audio.md` s2, s3). One *pass* of this model is
//! [`SyModel::isrs_per_pass`] audio frames (2, as one pass of the other chips is two frames,
//! `dsp-frame-rate.md`); each frame is:
//!
//! | Step | What | Source |
//! |---|---|---|
//! | 1 | up to `DM(0x92180)` queued ARM messages go into the FIFO ring behind a vector-13 interrupt | the DMA that fills the ring: ASSUMED one reset per batch (`sy-smp-audio.md` s3) |
//! | 2 | the 128-word staging buffer of every prepared voice is refilled | stand-in for the external-port DMA chain, D111 `refill`, ASSUMED policy |
//! | 3 | vector 17 is raised and run to its `RTI` | VERIFIED: one ISR is one stereo frame (`sy_voice_sine.scn`) |
//!
//! The frame is read back from the frame buffer word `DM(0x93590)` points at (default `0x932CA`).
use super::live_bus::BusTap;
use super::live_smp;
use crate::core::Core;
use std::collections::VecDeque;

/// FIFO reset interrupt (IVT word 52): the handler `0x1C03FD` sets `I0 = 0x93046`.
pub const FIFO_RESET_VECTOR: u8 = 13;
/// Audio interrupt (IVT word 68): the SY1SY2 frame ISR `0x1C05B9`.
pub const AUDIO_VECTOR: u8 = 17;
/// First word of the command FIFO ring (VERIFIED, listing `0x1C0638`).
pub const FIFO_BASE: u32 = 0x9_3046;
/// DMA write index of the FIFO (`DM(0x2400)`, read by the ISR at `0x1C0638`).
pub const FIFO_INDEX: u32 = 0x2400;
/// Maximum pairs the ISR takes per interrupt (`DM(0x92180)`, set by the start-up code).
pub const FIFO_CAP: u32 = 0x9_2180;
/// Voice records: word 0 of voice 0 (VERIFIED `sharc-ref` `script.rs`: the state word is `+0x14`), stride.
pub const REC_BASE: u32 = 0x9_2400;
pub const REC_STRIDE: u32 = 0x38;
/// Voice count (the 56 literals of the ARM record initialiser).
pub const VOICES: u32 = 56;
/// ARM message address of the Stage A state of voice `k`: `REC_BASE + 0x14 + 0x38 k` (`0x92414`).
pub const STATE_WORD: u32 = REC_BASE + 0x14;
/// Stage A states the ARM sends (VERIFIED `sy-smp-audio.md` s3).
pub const STATE_IDLE: u32 = 0x1C_07B0;
pub const STATE_RESET: u32 = 0x1C_0794;
pub const STATE_PREPARE: u32 = 0x1C_07CD;
pub const STATE_GO: u32 = 0x1C_084C;
/// Staging buffer base field of a voice record and its length (VERIFIED, `refill.rs`).
pub(crate) const STAGING_FIELD: u32 = 0x2F;
const STAGING_WORDS: u32 = 0x80;
/// ASSUMED (D111): a header word the unpacker accepts, so the ring is fed with control flow only.
pub const STAGING_FILL: u32 = (14 << 19) | 0x40;
/// Frame buffer pointer of the SY image and its first-half address (VERIFIED `sy-smp-audio.md` s4).
pub const FRAME_POINTER: u32 = 0x9_3590;
pub const FRAME_DEFAULT: u32 = 0x9_32CA;
/// Instruction limit of one interrupt (a 56 voice frame is well below).
pub const ISR_MAX_STEPS: u64 = 2_000_000;

/// Counters the `P` reply reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyStats {
    pub isrs: u64,
    pub resets: u64,
    pub pairs: u64,
    pub refills: u64,
}

/// State of the SY chip model beside the core.
#[derive(Debug)]
pub struct SyModel {
    pub isrs_per_pass: u32,
    /// Probe pcs counted inside the audio interrupt and their hit totals (`--probe`).
    pub probes: Vec<u32>,
    pub probe_hits: Vec<u64>,
    /// Register snapshots (`r0..7`, `i0..7`, `l0..1`) taken whenever the first probe pc is reached.
    pub snaps: Vec<String>,
    /// Command FIFO base: `FIFO_BASE` (SY1SY2) or `live_smp::FIFO_BASE` (SMP, no state words,
    /// no staging refill, output read from the post stage ring).
    fifo_base: u32,
    /// Offset the ISR adds to `DM(0x2400)` (`R3`): the index word holds `write address - bias`.
    index_bias: u32,
    /// Word holding the pairs-per-interrupt cap the ISR applies (`FIFO_CAP` for SY1SY2, `live_smp::FIFO_CAP` for SMP).
    cap_addr: u32,
    smp: bool,
    /// ARM pacing: at most this many queued pairs enter the FIFO per audio interrupt (`--fifo-pace`,
    /// D200; 0 = the whole queue up to the ISR cap, the earlier behaviour).
    pub pace: usize,
    /// SMP ring tap: read the half the pointer does NOT hold on entry (`H 1`, OQ41 ring half test).
    pub other_half: bool,
    /// Bus tap (`B ADDR`, D235): when set, the tapped frame is this DM bus pair instead of the
    /// ring (SMP) or frame buffer (SY). `None` = off, the default.
    pub bus_tap: Option<BusTap>,
    /// Side file of the bus tap (`B ADDR MODE PATH`): the bus frames (f32 little endian L R) go
    /// there and the ring tap goes on, so both are read in the same run.
    pub bus_file: Option<std::fs::File>,
    queue: VecDeque<(u32, u32)>,
    staged: [bool; VOICES as usize],
    pub stats: SyStats,
    frames: Vec<[u32; 2]>,
    error: Option<String>,
}

/// Runs `vector` to its `RTI`; the FIFO reset vector is unmasked for the call (as `sharc-ref irq`).
pub fn run_irq(core: &mut Core, vector: u8, max_steps: u64) -> Result<u64, String> {
    run_irq_probed(core, vector, max_steps, &[], &mut [], &mut Vec::new())
}

/// [`run_irq`] that also counts, per probe pc, how often an instruction at that pc is executed.
pub fn run_irq_probed(
    core: &mut Core,
    vector: u8,
    max_steps: u64,
    probes: &[u32],
    hits: &mut [u64],
    snaps: &mut Vec<String>,
) -> Result<u64, String> {
    let saved = core.imask;
    if vector <= 16 {
        core.imask |= 1 << vector;
    }
    if !core.raise_interrupt(vector) {
        core.imask = saved;
        return Err(format!("interrupt {vector} is invalid or in service"));
    }
    let start = core.steps;
    let mut taken = core.servicing().is_some();
    loop {
        if core.steps - start >= max_steps {
            let pc = core.pc;
            return Err(format!(
                "interrupt {vector} not finished in {max_steps} steps (pc {pc:#x})"
            ));
        }
        if let Some(i) = probes.iter().position(|&p| p == core.pc) {
            if let Some(h) = hits.get_mut(i) {
                *h += 1;
            }
            if i == 0 && snaps.len() < 256 {
                let f = |a: &[u32]| {
                    a.iter()
                        .map(|v| format!("{v:x}"))
                        .collect::<Vec<_>>()
                        .join(",")
                };
                snaps.push(format!(
                    "r={};i={};l={}",
                    f(&core.r[..8]),
                    f(&core.i[..8]),
                    f(&core.l[..2])
                ));
            }
        }
        core.step()
            .map_err(|f| format!("fault in interrupt {vector}: {f:?}"))?;
        taken |= core.servicing().is_some();
        if taken && core.servicing().is_none() {
            core.imask = saved;
            return Ok(core.steps - start);
        }
    }
}

impl SyModel {
    pub fn new(isrs_per_pass: u32) -> Self {
        Self {
            isrs_per_pass: isrs_per_pass.max(1),
            probes: Vec::new(),
            probe_hits: Vec::new(),
            snaps: Vec::new(),
            fifo_base: FIFO_BASE,
            index_bias: 0,
            cap_addr: FIFO_CAP,
            smp: false,
            pace: 0,
            other_half: false,
            bus_tap: None,
            bus_file: None,
            queue: VecDeque::new(),
            staged: [false; VOICES as usize],
            stats: SyStats::default(),
            frames: Vec::new(),
            error: None,
        }
    }

    /// The SMP (piano) chip: same interrupt-driven pass, its own FIFO and the ring tap (OQ41).
    pub fn smp(isrs_per_pass: u32) -> Self {
        Self {
            fifo_base: live_smp::FIFO_BASE,
            index_bias: live_smp::INDEX_BIAS,
            cap_addr: live_smp::FIFO_CAP,
            smp: true,
            other_half: false,
            ..Self::new(isrs_per_pass)
        }
    }

    /// Sets the offset the ISR adds to `DM(0x2400)` (`--index-bias`, opt-in for the SY1SY2 chip,
    /// D206: its init sets `R3 = 0x80000` like the SMP image, the model stored the plain address).
    pub fn with_index_bias(mut self, bias: u32) -> Self {
        self.index_bias = bias;
        self
    }

    /// Queues one ARM message (an opcode 0 pair: store `value` at DM `addr`). A Stage A state
    /// message also switches the ring refill of its voice (prepare and go on, idle and reset off).
    pub fn send(&mut self, addr: u32, value: u32) {
        if let Some(k) = addr
            .checked_sub(STATE_WORD)
            .filter(|_| !self.smp)
            .filter(|o| o % REC_STRIDE == 0 && o / REC_STRIDE < VOICES)
            .map(|o| o / REC_STRIDE)
        {
            match value {
                STATE_PREPARE | STATE_GO => self.staged[k as usize] = true,
                STATE_IDLE | STATE_RESET => self.staged[k as usize] = false,
                _ => {}
            }
        }
        self.queue.push_back((addr & 0x0FFF_FFFF, value));
    }

    /// Messages not yet in the FIFO.
    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    /// Voices whose staging buffer is refilled.
    pub fn staged_voices(&self) -> usize {
        self.staged.iter().filter(|s| **s).count()
    }

    /// The first error of the last pass (taken).
    pub fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }

    /// Frame words `[left, right]` the last pass produced, flattened (two words per frame).
    pub fn level_words(&self) -> Vec<u32> {
        self.frames.iter().flatten().copied().collect()
    }

    /// FIFO reset plus up to `DM(0x92180)` queued pairs written behind it (step 1).
    pub(crate) fn feed_fifo(&mut self, core: &mut Core) -> Result<u64, String> {
        let cap = core.dm_read(self.cap_addr).unwrap_or(0) as usize;
        let mut n = self.queue.len().min(cap);
        if self.pace > 0 {
            n = n.min(self.pace);
        }
        if n == 0 {
            return Ok(0);
        }
        let steps = run_irq(core, FIFO_RESET_VECTOR, ISR_MAX_STEPS)?;
        let mut at = self.fifo_base;
        for (addr, value) in self.queue.drain(..n) {
            let w = core
                .dm_write(at, addr)
                .and_then(|()| core.dm_write(at + 1, value));
            w.map_err(|a| format!("bad FIFO address {a:#x}"))?;
            at += 2;
        }
        core.dm_write(FIFO_INDEX, at.wrapping_sub(self.index_bias))
            .map_err(|a| format!("bad FIFO index address {a:#x}"))?;
        self.stats.resets += 1;
        self.stats.pairs += n as u64;
        Ok(steps)
    }

    /// Refills the staging buffers of the prepared voices (step 2).
    pub(crate) fn refill(&mut self, core: &mut Core) -> Result<(), String> {
        for k in (0..VOICES).filter(|k| self.staged[*k as usize]) {
            let base = core
                .dm_read(REC_BASE + REC_STRIDE * k + STAGING_FIELD)
                .unwrap_or(0);
            if base == 0 {
                continue; // before the prepare state ran: no buffer yet
            }
            for w in 0..STAGING_WORDS {
                core.dm_write(base + w, STAGING_FILL)
                    .map_err(|a| format!("refill: bad address {a:#x}"))?;
            }
            self.stats.refills += 1;
        }
        Ok(())
    }

    /// One audio frame: FIFO batch, refill, the audio interrupt, the frame read back.
    fn frame(&mut self, core: &mut Core) -> Result<u64, String> {
        let mut steps = self.feed_fifo(core)?;
        self.refill(core)?;
        let mut half = live_smp::ring_half(core);
        if self.smp && self.other_half {
            half = live_smp::other_half(half);
        }
        steps += run_irq_probed(
            core,
            AUDIO_VECTOR,
            ISR_MAX_STEPS,
            &self.probes,
            &mut self.probe_hits,
            &mut self.snaps,
        )?;
        self.stats.isrs += 1;
        if let Some(tap) = self.bus_tap {
            let bus = tap.frame(core);
            match &mut self.bus_file {
                Some(f) => {
                    let bytes: Vec<u8> = bus.iter().flat_map(|w| w.to_le_bytes()).collect();
                    if let Err(e) = std::io::Write::write_all(f, &bytes) {
                        self.error.get_or_insert(format!("bus file: {e}"));
                    }
                }
                None => {
                    self.frames.push(bus);
                    return Ok(steps);
                }
            }
        }
        if self.smp {
            self.frames.extend(live_smp::ring_frames(core, half));
            return Ok(steps);
        }
        let p = core.dm_read(FRAME_POINTER).unwrap_or(0);
        let at = if (0x9_0000..0x10_0000).contains(&p) {
            p
        } else {
            FRAME_DEFAULT
        };
        self.frames.push([
            core.dm_read(at).unwrap_or(0),
            core.dm_read(at + 1).unwrap_or(0),
        ]);
        Ok(steps)
    }

    /// The frames (float bits `[L, R]`) the last pass tapped, one per audio interrupt.
    pub fn frames(&self) -> &[[u32; 2]] {
        &self.frames
    }

    /// One pass (`isrs_per_pass` frames): `(steps, ok)`; an error is kept for the `P` reply.
    pub fn pass(&mut self, core: &mut Core) -> (u64, bool) {
        self.frames.clear();
        let mut steps = 0;
        for _ in 0..self.isrs_per_pass {
            match self.frame(core) {
                Ok(s) => steps += s,
                Err(e) => {
                    self.error.get_or_insert(e);
                    return (steps, false);
                }
            }
        }
        (steps, true)
    }

    /// The `sy=` token of the `P` reply (no spaces).
    pub fn report(&self) -> String {
        let s = self.stats;
        let mut out = format!(
            "sy=isrs:{},resets:{},pairs:{},queued:{},refills:{},staged:{}",
            s.isrs,
            s.resets,
            s.pairs,
            self.queue.len(),
            s.refills,
            self.staged_voices()
        );
        if !self.probes.is_empty() {
            let list: Vec<String> = self
                .probes
                .iter()
                .zip(&self.probe_hits)
                .map(|(p, h)| format!("{p:x}:{h}"))
                .collect();
            out.push_str(&format!(" ihits={}", list.join(",")));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_smp_model_has_its_own_fifo_bias_and_cap_word_the_sy_model_none() {
        let sy = SyModel::new(2);
        assert_eq!((sy.index_bias, sy.cap_addr), (0, FIFO_CAP));
        let smp = SyModel::smp(2);
        assert_eq!(smp.index_bias, live_smp::INDEX_BIAS);
        assert_eq!(smp.cap_addr, live_smp::FIFO_CAP);
        assert_eq!(smp.fifo_base, live_smp::FIFO_BASE);
    }

    #[test]
    fn the_index_bias_is_settable_on_the_sy_model_and_defaults_off() {
        assert_eq!(SyModel::new(2).index_bias, 0);
        assert_eq!(
            SyModel::new(2).with_index_bias(0x8_0000).index_bias,
            live_smp::INDEX_BIAS
        );
        let smp = SyModel::smp(2).with_index_bias(0);
        assert_eq!((smp.index_bias, smp.cap_addr), (0, live_smp::FIFO_CAP));
    }

    #[test]
    fn the_report_lists_probe_hits_only_when_probes_are_set() {
        let mut m = SyModel::new(1);
        assert!(!m.report().contains("ihits="));
        m.probes = vec![0x1C_05D7, 0x1C_0B08];
        m.probe_hits = vec![3, 0];
        assert!(
            m.report().ends_with(" ihits=1c05d7:3,1c0b08:0"),
            "{}",
            m.report()
        );
    }
}
