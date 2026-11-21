//! Tests of `live_sy.rs` (the SY / SMP interrupt-driven pass model), split out to keep it under the
//! module size rule.
use super::live_smp;
use super::live_sy::STAGING_FIELD;
use super::live_sy::*;
use crate::core::Core;

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
    use super::live::Live;
    use super::pass::PassConfig;
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

#[test]
fn the_smp_model_uses_its_own_fifo_never_stages_and_taps_the_ring() {
    let mut c = core_with_rti_vectors();
    c.dm_write(live_smp::FIFO_CAP, 4).unwrap();
    c.dm_write(live_smp::RING_POINTER, live_smp::RING_HALVES[1])
        .unwrap();
    c.dm_write(live_smp::RING_HALVES[1], 0.5f32.to_bits())
        .unwrap();
    c.dm_write(live_smp::RING_HALVES[1] + 11, (-1.0f32).to_bits())
        .unwrap();
    let mut m = SyModel::smp(1);
    m.send(STATE_WORD, STATE_PREPARE); // an SY state word is a plain store on the SMP
    m.send(0x9_3936, 0x3F80_0000);
    assert_eq!(m.staged_voices(), 0);
    let (steps, ok) = m.pass(&mut c);
    assert!(ok && steps > 0, "{:?}", m.take_error());
    assert_eq!(c.dm_read(live_smp::FIFO_BASE + 2), Ok(0x9_3936));
    assert_eq!(c.dm_read(FIFO_BASE), Ok(0), "the SY FIFO is untouched");
    // the ISR adds R3 = 0x80000 to the index word: it holds `write address - bias` (OQ41)
    assert_eq!(
        c.dm_read(FIFO_INDEX),
        Ok(live_smp::FIFO_BASE + 4 - live_smp::INDEX_BIAS)
    );
    let w = m.level_words();
    assert_eq!(w.len(), 4, "two stereo frames per interrupt");
    assert_eq!(f32::from_bits(w[0]), 0.5);
    assert_eq!(f32::from_bits(w[3]), -1.0);
}

#[test]
fn the_smp_batch_is_capped_by_its_own_word_not_the_sy_one() {
    let mut c = core_with_rti_vectors();
    c.dm_write(FIFO_CAP, 1).unwrap(); // the SY1SY2 cap word must not limit the SMP
    c.dm_write(live_smp::FIFO_CAP, 3).unwrap();
    let mut m = SyModel::smp(1);
    for k in 0..5 {
        m.send(0x9_5000 + k, k);
    }
    let (_, ok) = m.pass(&mut c);
    assert!(ok);
    assert_eq!(m.stats.pairs, 3);
    assert_eq!(m.queued(), 2);
}

#[test]
fn probes_count_executions_and_snapshots_record_registers() {
    // vector 17 = NOP (3 parcels), RTI: the RTI is the probe
    let mut image = vec![0u8; 0x24_0000 + 2 * 12 * 20];
    for v in 0..20usize {
        let at = 0x24_0000 + 2 * 12 * v;
        let parcels: &[u16] = if v == 17 {
            &[0, 0, 0, 0x0bbe, 1, 1]
        } else {
            &[0x0bbe, 1, 1]
        };
        for (k, p) in parcels.iter().enumerate() {
            image[at + 2 * k..at + 2 * k + 2].copy_from_slice(&p.to_le_bytes());
        }
    }
    let mut c = Core::new(&image);
    c.pc = 0x12_0000 + 3 * 100;
    c.mode1 |= 1 << 12;
    c.write_ureg(0x7D, 1 << 10);
    let mut m = SyModel::new(2);
    m.probes = vec![0x12_0000 + 12 * 17 + 3, 0x12_0000]; // the RTI of vector 17, a pc never run
    m.probe_hits = vec![0, 0];
    let (_, ok) = m.pass(&mut c);
    assert!(ok, "{:?}", m.take_error());
    assert_eq!(m.probe_hits, vec![2, 0], "two audio interrupts per pass");
    assert_eq!(m.snaps.len(), 2, "one snapshot per hit of the first probe");
    assert!(
        m.report().ends_with("ihits=1200cf:2,120000:0"),
        "{}",
        m.report()
    );
}
