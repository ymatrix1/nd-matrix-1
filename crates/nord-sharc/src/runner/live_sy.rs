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
const STAGING_FIELD: u32 = 0x2F;
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
    queue: VecDeque<(u32, u32)>,
    staged: [bool; VOICES as usize],
    pub stats: SyStats,
    frames: Vec<[u32; 2]>,
    error: Option<String>,
}

/// Runs `vector` to its `RTI`; the FIFO reset vector is unmasked for the call (as `sharc-ref irq`).
pub fn run_irq(core: &mut Core, vector: u8, max_steps: u64) -> Result<u64, String> {
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
            queue: VecDeque::new(),
            staged: [false; VOICES as usize],
            stats: SyStats::default(),
            frames: Vec::new(),
            error: None,
        }
    }

    /// Queues one ARM message (an opcode 0 pair: store `value` at DM `addr`). A Stage A state
    /// message also switches the ring refill of its voice (prepare and go on, idle and reset off).
    pub fn send(&mut self, addr: u32, value: u32) {
        if let Some(k) = addr
            .checked_sub(STATE_WORD)
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
    fn feed_fifo(&mut self, core: &mut Core) -> Result<u64, String> {
        let cap = core.dm_read(FIFO_CAP).unwrap_or(0) as usize;
        let n = self.queue.len().min(cap);
        if n == 0 {
            return Ok(0);
        }
        let steps = run_irq(core, FIFO_RESET_VECTOR, ISR_MAX_STEPS)?;
        let mut at = FIFO_BASE;
        for (addr, value) in self.queue.drain(..n) {
            let w = core
                .dm_write(at, addr)
                .and_then(|()| core.dm_write(at + 1, value));
            w.map_err(|a| format!("bad FIFO address {a:#x}"))?;
            at += 2;
        }
        core.dm_write(FIFO_INDEX, at)
            .map_err(|a| format!("bad FIFO index address {a:#x}"))?;
        self.stats.resets += 1;
        self.stats.pairs += n as u64;
        Ok(steps)
    }

    /// Refills the staging buffers of the prepared voices (step 2).
    fn refill(&mut self, core: &mut Core) -> Result<(), String> {
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
        steps += run_irq(core, AUDIO_VECTOR, ISR_MAX_STEPS)?;
        self.stats.isrs += 1;
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
        format!(
            "sy=isrs:{},resets:{},pairs:{},queued:{},refills:{},staged:{}",
            s.isrs,
            s.resets,
            s.pairs,
            self.queue.len(),
            s.refills,
            self.staged_voices()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A core whose vectors are all `RTI` at once (IVT at short word `0x120000`, 12 parcels
    /// per vector), so a vector run only counts the interrupt.
    fn core_with_rti_vectors() -> Core {
        let mut image = vec![0u8; 0x24_0000 + 2 * 12 * 20];
        for v in 0..20usize {
            let at = 0x24_0000 + 2 * 12 * v;
            for (k, p) in [0x0bbeu16, 1, 1].iter().enumerate() {
                image[at + 2 * k..at + 2 * k + 2].copy_from_slice(&p.to_le_bytes());
            }
        }
        let mut c = Core::new(&image);
        c.pc = 0x12_0000 + 3 * 100;
        c.mode1 |= 1 << 12; // MODE1.IRPTEN
        c.write_ureg(0x7D, 1 << 10); // LIRPTL mask bit of vector 17 (P6I), as the firmware sets it
        c
    }

    #[test]
    fn state_messages_switch_the_refill_per_voice() {
        let mut m = SyModel::new(2);
        m.send(STATE_WORD + 3 * REC_STRIDE, STATE_PREPARE);
        assert_eq!(m.staged_voices(), 1);
        m.send(STATE_WORD + 3 * REC_STRIDE, STATE_GO);
        assert_eq!(m.staged_voices(), 1);
        m.send(STATE_WORD + 3 * REC_STRIDE, STATE_RESET);
        assert_eq!(m.staged_voices(), 0);
        m.send(STATE_WORD + 1, STATE_PREPARE); // not a state word
        m.send(STATE_WORD + VOICES * REC_STRIDE, STATE_PREPARE); // voice 56 does not exist
        assert_eq!((m.staged_voices(), m.queued()), (0, 5));
    }

    #[test]
    fn a_batch_is_capped_by_the_isr_pair_limit_and_resets_the_fifo() {
        let mut c = core_with_rti_vectors();
        c.dm_write(FIFO_CAP, 2).unwrap();
        let mut m = SyModel::new(1);
        for k in 0..5 {
            m.send(0x9_5000 + k, 0x100 + k);
        }
        m.feed_fifo(&mut c).unwrap();
        // two pairs behind the base, the index after them, three still queued, one reset
        let w: Vec<u32> = (0..5).map(|k| c.dm_read(FIFO_BASE + k).unwrap()).collect();
        assert_eq!(&w[..4], &[0x9_5000, 0x100, 0x9_5001, 0x101]);
        assert_eq!(c.dm_read(FIFO_INDEX), Ok(FIFO_BASE + 4));
        assert_eq!((m.queued(), m.stats.resets, m.stats.pairs), (3, 1, 2));
        assert_eq!(c.interrupts_taken(), 1);
        m.feed_fifo(&mut c).unwrap();
        assert_eq!((m.queued(), m.stats.pairs), (1, 4));
    }

    #[test]
    fn no_cap_or_no_message_means_no_fifo_interrupt() {
        let mut c = core_with_rti_vectors();
        let mut m = SyModel::new(1);
        m.send(0x9_5000, 1);
        assert_eq!(m.feed_fifo(&mut c), Ok(0), "cap 0: nothing is consumed");
        c.dm_write(FIFO_CAP, 4).unwrap();
        let mut empty = SyModel::new(1);
        assert_eq!(empty.feed_fifo(&mut c), Ok(0));
        assert_eq!(c.interrupts_taken(), 0);
    }

    #[test]
    fn refill_fills_only_prepared_voices_with_a_buffer() {
        let mut c = core_with_rti_vectors();
        let mut m = SyModel::new(1);
        m.send(STATE_WORD, STATE_PREPARE);
        m.send(STATE_WORD + REC_STRIDE, STATE_PREPARE);
        c.dm_write(REC_BASE + STAGING_FIELD, 0x9_3FF7).unwrap();
        // voice 1 has no staging base yet: skipped
        m.refill(&mut c).unwrap();
        assert_eq!(c.dm_read(0x9_3FF7), Ok(STAGING_FILL));
        assert_eq!(c.dm_read(0x9_3FF7 + 0x7F), Ok(STAGING_FILL));
        assert_eq!(c.dm_read(0x9_3FF7 + 0x80), Ok(0));
        assert_eq!(m.stats.refills, 1);
    }

    #[test]
    fn a_pass_runs_isrs_per_pass_audio_interrupts_and_reads_the_frame() {
        let mut c = core_with_rti_vectors();
        c.dm_write(FRAME_POINTER, 0x9_4000).unwrap();
        c.dm_write(0x9_4000, 0.5f32.to_bits()).unwrap();
        c.dm_write(0x9_4001, (-0.25f32).to_bits()).unwrap();
        let mut m = SyModel::new(2);
        let (steps, ok) = m.pass(&mut c);
        assert!(ok && steps > 0, "{:?}", m.take_error());
        assert_eq!((m.stats.isrs, c.interrupts_taken()), (2, 2));
        assert_eq!(m.level_words().len(), 4);
        assert_eq!(f32::from_bits(m.level_words()[0]), 0.5);
        assert!(m
            .report()
            .starts_with("sy=isrs:2,resets:0,pairs:0,queued:0,"));
    }

    #[test]
    fn a_stuck_interrupt_is_reported_not_hung() {
        let mut c = core_with_rti_vectors();
        // vector 17 at a pc that never returns: overwrite its RTI with NOPs is not needed, ask
        // for a step budget of zero instead
        assert!(run_irq(&mut c, AUDIO_VECTOR, 0).is_err());
        assert!(run_irq(&mut c, 99, 10).is_err(), "invalid vector");
    }

    #[test]
    fn the_live_protocol_queues_messages_and_reports_frames_and_counters() {
        use super::super::live::Live;
        use super::super::pass::PassConfig;
        let mut c = core_with_rti_vectors();
        c.dm_write(FIFO_CAP, 4).unwrap();
        c.dm_write(FRAME_POINTER, 0x9_4000).unwrap();
        c.dm_write(0x9_4000, 0.5f32.to_bits()).unwrap();
        c.dm_write(0x9_4001, (-0.75f32).to_bits()).unwrap();
        let cfg = PassConfig {
            trigger: 0,
            idle_pc: 0,
            max_steps: 10,
            irq: None,
        };
        let mut l = Live::new(c, cfg, None, (2, 0), vec![]).with_sy(SyModel::new(2));
        assert_eq!(l.exec("M 0x95000 0x1234").unwrap(), None);
        // the message waits in the queue: the data word is untouched until a pass moved it
        // into the FIFO (the stub ISR here is an RTI, so it is never stored)
        assert_eq!(l.exec("D 0x95000 1").unwrap().unwrap(), "D 95000 0");
        let r = l.exec("P 1").unwrap().unwrap();
        assert!(r.starts_with("P req=1 idle=1 "), "{r}");
        assert!(
            r.contains("cnt=4") && r.contains("peak=7.500000000e-1"),
            "{r}"
        );
        assert!(r.contains("sy=isrs:2,resets:1,pairs:1,queued:0,"), "{r}");
        assert!(!r.contains("error="), "{r}");
        assert_eq!(
            l.exec("D 0x93046 2").unwrap().unwrap(),
            "D 93046 95000 1234"
        );
    }
}
