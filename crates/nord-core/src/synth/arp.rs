//! Arpeggiator / gate engine for `SynthLayer`.

use super::arp_order::pool_position;
use super::arp_pool::{firmware_pool, firmware_span, PoolNote, POOL_MAX};
use super::*;

impl SynthLayer {
    /// Advances the arpeggiator engine and generates notes when active.
    pub(super) fn tick_arpeggiator(&mut self, inv_fs: f32) {
        if self.arp_mode != ArpMode::Gate {
            self.perf.gate_level = 1.0;
        }
        if self.arp_mode == ArpMode::Off {
            return;
        }
        if self.arp_mode == ArpMode::Gate {
            self.tick_gate(inv_fs);
            return;
        }

        let mut held_list = [0u8; 128];
        let mut count = 0;
        for (n, &held) in self.held_keys.iter().enumerate() {
            if held {
                held_list[count] = n as u8;
                count += 1;
            }
        }

        if count == 0 {
            self.release_arp_notes();
            self.perf.tie_out = false;
            self.perf.tied_in = false;
            self.arp_step = 0;
            return;
        }

        // Arp speed: a 1/8 note at arp_rate_bpm, or a Master Clock subdivision when synced.
        let step_sec = self.step_seconds();
        self.arp_sample_counter += inv_fs;

        if self.arp_sample_counter < step_sec {
            if self.perf.pattern_on {
                self.pattern_release(step_sec);
            } else {
                self.plain_gate_release(step_sec);
            }
        } else {
            self.arp_sample_counter -= step_sec;

            // Release the previous arp note, unless a `Q` step ties it into this one.
            let keep = self.pattern_keeps_notes();
            if !keep {
                self.release_arp_notes();
            }
            if self.pattern_rest() {
                if keep && self.perf.cur_len == 0 {
                    self.release_arp_notes();
                }
                return;
            }

            if self.arp_mode == ArpMode::Poly {
                for (slot, &note) in held_list
                    .iter()
                    .take(count.min(MAX_SYNTH_VOICES))
                    .enumerate()
                {
                    let velocity = self.accent_velocity(self.held_velocities[note as usize]);
                    self.perf.voice_pan = self.step_pan();
                    self.trigger_voice(note, velocity, self.held_gains[note as usize]);
                    self.perf.voice_pan = 0.0;
                    self.arp_active_notes[slot] = Some(note);
                }
                self.arp_step = (self.arp_step + 1) % count.max(1);
                return;
            }

            let zig = self.perf.zigzag;
            if self.firmware_walk_applies() {
                self.trigger_firmware_walk(&held_list[..count]);
                return;
            }

            // Pick next note
            let oct_range = self.arp_octave_range.clamp(1, 4) as usize;
            let extra = extra_notes(&held_list[..count], self.perf.range_extra_thirds);
            let pool = count * oct_range + extra;
            let cycle = pool;
            // ZIG ZAG walks the pool two forward, one back; the step counter then runs over
            // two periods.
            let step = if zig {
                zigzag_index(self.arp_step, pool)
            } else {
                self.arp_step % cycle
            };

            let (note_in_chord, oct_offset) = match self.arp_mode {
                ArpMode::Up => pool_position(step, count),
                ArpMode::Down if extra > 0 => pool_position(pool - 1 - step, count),
                ArpMode::Down => (
                    (count - 1) - (step % count),
                    (oct_range - 1 - (step / count)) as u8,
                ),
                ArpMode::UpDown if extra > 0 => {
                    let cycle_len = (pool * 2).saturating_sub(2).max(1);
                    let sub_step = step % cycle_len;
                    let pos = if sub_step < pool {
                        sub_step
                    } else {
                        cycle_len - sub_step
                    };
                    pool_position(pos, count)
                }
                ArpMode::UpDown => {
                    let cycle_len = (count * 2).saturating_sub(2).max(1);
                    let sub_step = step % cycle_len;
                    let n_idx = if sub_step < count {
                        sub_step
                    } else {
                        cycle_len - sub_step
                    };
                    (n_idx, (step / count) as u8)
                }
                ArpMode::Random => {
                    self.arp_rng = self
                        .arp_rng
                        .wrapping_mul(1_664_525)
                        .wrapping_add(1_013_904_223);
                    if extra > 0 {
                        pool_position((self.arp_rng as usize) % pool, count)
                    } else {
                        (
                            (self.arp_rng as usize) % count,
                            ((self.arp_rng >> 16) as usize % oct_range) as u8,
                        )
                    }
                }
                ArpMode::Off | ArpMode::Poly | ArpMode::Gate => unreachable!(),
            };

            let next_note = (held_list[note_in_chord] + oct_offset * 12).min(127);
            self.arp_active_notes[0] = Some(next_note);
            self.arp_step = (self.arp_step + 1) % if zig { pool * 2 } else { cycle };

            let source_note = held_list[note_in_chord] as usize;
            let velocity = self.accent_velocity(self.held_velocities[source_note]);
            self.perf.voice_pan = self.step_pan();
            self.trigger_voice(next_note, velocity, self.held_gains[source_note]);
            self.perf.voice_pan = 0.0;
        }
    }

    /// The firmware walk plays this step: UP, DOWN or UP/DOWN without ZIG ZAG (RANDOM, ZIG ZAG and
    /// Poly were not captured and keep the engine walk).
    fn firmware_walk_applies(&self) -> bool {
        self.arp_firmware_walk
            && !self.perf.zigzag
            && matches!(self.arp_mode, ArpMode::Up | ArpMode::Down | ArpMode::UpDown)
    }

    /// Stored RANGE value the firmware span rule reads: the decoded raw while it still projects
    /// onto the engine's range (octaves and thirds), else the raw of the nearest detent.
    fn firmware_range_raw(&self) -> f32 {
        let thirds = u32::from(self.arp_octave_range.clamp(1, 4) - 1) * 3
            + u32::from(self.perf.range_extra_thirds);
        let stored = (f32::from(self.arp_range_raw) / 127.0 * 9.0).round() as u32;
        if stored == thirds {
            f32::from(self.arp_range_raw)
        } else {
            thirds as f32 * 127.0 / 9.0
        }
    }

    /// Plays the next note of the firmware walk over the pool of `held` (ascending keys).
    fn trigger_firmware_walk(&mut self, held: &[u8]) {
        let mut buf = [PoolNote::default(); POOL_MAX];
        let span = firmware_span(self.firmware_range_raw());
        let len = firmware_pool(held, span, &mut buf).max(1);
        let cycle = if self.arp_mode == ArpMode::UpDown {
            (len * 2).saturating_sub(2).max(1)
        } else {
            len
        };
        let step = self.arp_step % cycle;
        let index = match self.arp_mode {
            ArpMode::Down => len - 1 - step,
            ArpMode::UpDown if step >= len => cycle - step,
            _ => step,
        };
        let PoolNote { note, source } = buf[index.min(len - 1)];
        self.arp_active_notes[0] = Some(note);
        self.arp_step = (self.arp_step + 1) % cycle;
        let velocity = self.accent_velocity(self.held_velocities[source as usize]);
        self.perf.voice_pan = self.step_pan();
        self.trigger_voice(note, velocity, self.held_gains[source as usize]);
        self.perf.voice_pan = 0.0;
    }

    pub(super) fn release_arp_notes(&mut self) {
        for active in &mut self.arp_active_notes {
            if let Some(note) = active.take() {
                for voice in &mut self.voices {
                    if voice.active && voice.note == note {
                        voice.gate = false;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arpeggiator_generation() {
        let mut synth = SynthLayer::new(48_000.0);
        synth.arp_mode = ArpMode::Up;
        synth.arp_rate_bpm = 480.0;
        synth.note_on(60, 100);
        synth.note_on(64, 100);
        synth.note_on(67, 100);

        let mut energy = 0.0f32;
        for _ in 0..24000 {
            let [l, r] = synth.render_frame();
            energy += l * l + r * r;
        }
        assert!(energy > 0.05, "Arpeggiator must output active sound energy");
        assert!(synth.arp_step > 0, "Arpeggiator step must advance");
    }

    #[test]
    fn arpeggiator_steps_every_thirty_over_bpm_seconds() {
        // VERIFIED step length (`arp_rate.rs`): a step is a 1/8 note at the displayed tempo, so
        // 120 BPM steps every 250 ms = 12000 samples at 48 kHz (not the 16th it used to be).
        let mut synth = SynthLayer::new(48_000.0);
        synth.arp_mode = ArpMode::Up;
        synth.arp_rate_bpm = 120.0;
        synth.note_on(60, 100);
        synth.note_on(64, 100);
        let mut changes = Vec::new();
        let mut last = synth.arp_step;
        for n in 0..40_000usize {
            synth.render_frame();
            if synth.arp_step != last {
                changes.push(n);
                last = synth.arp_step;
            }
        }
        assert_eq!(changes.len(), 3, "steps in 40000 samples: {changes:?}");
        for pair in changes.windows(2) {
            assert!((pair[1] - pair[0]).abs_diff(12_000) <= 1, "{changes:?}");
        }
        assert!(changes[0].abs_diff(12_000) <= 1, "{changes:?}");
    }

    #[test]
    fn poly_arpeggiator_triggers_held_chord_simultaneously() {
        let mut synth = SynthLayer::new(48_000.0);
        synth.arp_mode = ArpMode::Poly;
        synth.arp_rate_bpm = 480.0;
        synth.note_on(60, 70);
        synth.note_on(64, 90);
        synth.note_on(67, 110);
        for _ in 0..3_200 {
            synth.render_frame();
        }
        for note in [60, 64, 67] {
            assert!(synth
                .voices
                .iter()
                .any(|voice| voice.active && voice.gate && voice.note == note));
        }
    }

    #[test]
    fn arpeggiator_uses_allocator_without_overwriting_an_active_voice() {
        let mut synth = SynthLayer::legacy(48_000.0);
        synth.note_on(40, 100);
        synth.note_off(40);
        synth.arp_mode = ArpMode::Up;
        synth.arp_rate_bpm = 480.0;
        synth.note_on(60, 100);
        synth.note_on(64, 100);
        for _ in 0..3_200 {
            synth.render_frame();
        }
        assert_eq!(synth.voices[0].note, 40);
        assert!(!synth.voices[0].gate);
        assert!(synth
            .voices
            .iter()
            .any(|voice| voice.active && voice.gate && voice.note == 60));
    }

    /// Notes the arpeggiator picks over `steps` steps at 100 BPM sixteenth steps.
    fn picked_notes(synth: &mut SynthLayer, steps: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut last = None;
        for _ in 0..(steps * 7_300) {
            synth.render_frame();
            if synth.arp_active_notes[0] != last {
                last = synth.arp_active_notes[0];
                out.extend(last);
            }
        }
        out
    }

    fn updown_layer(keys: &[u8], firmware_walk: bool) -> SynthLayer {
        let mut synth = SynthLayer::new(48_000.0);
        synth.arp_mode = ArpMode::UpDown;
        synth.arp_octave_range = 2;
        synth.arp_rate_bpm = 480.0;
        synth.arp_firmware_walk = firmware_walk;
        for &k in keys {
            synth.note_on(k, 100);
        }
        synth
    }

    #[test]
    fn firmware_walk_mirrors_over_the_whole_pool_with_the_top_root() {
        // ARM capture (arp-fw-events-a16.json, layer C, 2 octaves): 64 67 72 76 79 84 79 76 72 67.
        let seq = picked_notes(&mut updown_layer(&[60, 64, 67], true), 14);
        assert_eq!(
            seq[..13],
            [60, 64, 67, 72, 76, 79, 84, 79, 76, 72, 67, 64, 60]
        );
    }

    #[test]
    fn firmware_walk_single_key_visits_three_octaves_and_back() {
        // ARM capture: single key 60 plays 72 84 72 60 72 84 72 60 (cycle 60 72 84 72).
        let seq = picked_notes(&mut updown_layer(&[60], true), 8);
        assert_eq!(seq[..8], [60, 72, 84, 72, 60, 72, 84, 72]);
    }

    #[test]
    fn engine_walk_is_unchanged_without_the_flag() {
        let seq = picked_notes(&mut updown_layer(&[60, 64, 67], false), 7);
        assert_eq!(seq[..6], [60, 64, 67, 76, 72, 76]);
    }
}
