//! Firmware voice-gain slew of one piano stroke voice: the SMP record update of
//! [`super::smp_slew`] driven at the SMP update rate, with the initial values the ARM and the SMP
//! command handler leave in the record at note-on and the reset the ARM writes at release.
//!
//! Evidence: `docs/research/smp-slew.md` section 6 (ARM message lists of a note-on and note-off
//! for four sounds at four velocities, replayed into the `sharc-ref` oracle running the SMP image).
//! The type is `Copy` and holds no heap data, so voices can carry it on the audio path.

use super::smp_slew::SlewRecord;

/// VERIFIED (oracle replay of the note-on message list, 4 velocities, `tests::note_on_gains`):
/// the SMP voice-start handler leaves `F-2 = 0.25` (the ARM word `DM 0x9252D`), `F-1 = 0`,
/// `F2 = 0`, `F3 = 0.5`, `F11 = F12 = 0`, `F13 = F14 = F16 = F18 = 1.0` and the multipliers
/// `F19 = F20 = 0x70000000` (0.875); `F15` is the level word (`DM 0x9252A`) and `F17` the rate
/// word (`DM 0x9252C`).
const RAMP: f32 = 0.25;
const RAMP_CEIL: f32 = 0.5;
const GAIN_MUL: i32 = 0x7000_0000;
/// VERIFIED (ARM note-off list, 4 velocities, identical): the release write resets
/// `F11 = F12 = 0`, `F14 = 1.0`, `F16 = 0x3F7FBB6E` of the main voice record.
const RELEASE_Y_STATE: u32 = 0x3F7F_BB6E;
/// One Q31 unit.
const Q31: f32 = 2_147_483_648.0;
/// Below this gain (about -120 dB of full scale) a released voice is done.
const DONE_GAIN: f32 = 1e-6;

/// Rate word (`F17`) the ARM stores for a note-on: `1 - d / 2^24` with
/// `d = 2 * round(-decay * 2^(semitones / 12))`, `decay` the big-endian `i16` at stroke record
/// byte `0x3C` ([`super::npno_level::VoiceLevel::decay`]) and `semitones` the playback pitch above
/// the stroke root (`note - root + cents / 100`). A non-negative `decay` gives no decay.
/// VERIFIED: 48 of 48 Felt Upright firmware runs (6 notes x 8 velocities, all on the root pitch)
/// and 7 of 7 Velvet Grand points (note 57 is 1 semitone above its root: `d = 356 / 306 / 190`
/// from `decay = -168 / -144 / -90`), `tools/arm_smp_slew_init.py rates`, tests
/// `rate_law_matches_the_firmware_words`. ASSUMED: nearest rounding and the cents term (what would
/// confirm it: ARM runs on notes with a tuning offset or 2 to 11 semitones above the root).
pub(super) fn rate_from_record(decay: i16, semitones: f32) -> f32 {
    let ratio = 2.0f32.powf(semitones / 12.0);
    let d = 2.0 * (-f32::from(decay) * ratio).round().max(0.0);
    1.0 - d / 16_777_216.0
}

/// VERIFIED (ARM note-off message list, `docs/research/smp-slew.md` s6): the key-up sample record
/// gets the rate word `F17 = 0x3F7FF38E` (time constant 0.24 s at 43,402.34 / 2 updates per
/// second; the doc's 0.23 s is the same word read at 43.4 kHz) for every velocity and sound.
const RELEASE_SAMPLE_RATE: u32 = 0x3F7F_F38E;
/// VERIFIED (same list): the key-up record's level word `F15` is `0x3F7FFFFE` (1.0) at velocities
/// 20, 64 and 100 and `0x3F6C1CEA` (0.922) at velocity 127.
const RELEASE_SAMPLE_LEVEL: u32 = 0x3F7F_FFFE;
const RELEASE_SAMPLE_LEVEL_127: u32 = 0x3F6C_1CEA;

/// Level word of the key-up sample record for a velocity.
/// ASSUMED between the verified points: 1.0 up to velocity 100, then linear to 0.922 at 127 (what
/// would confirm it: the ARM note-off lists at velocities 101 to 126).
pub(super) fn release_sample_level(velocity: u8) -> f32 {
    let hi = f32::from_bits(RELEASE_SAMPLE_LEVEL);
    let lo = f32::from_bits(RELEASE_SAMPLE_LEVEL_127);
    let t = ((f32::from(velocity) - 100.0) / 27.0).clamp(0.0, 1.0);
    hi + (lo - hi) * t
}

/// Time constant of the key-up record's decay on the engine envelope path: `F17` per update, one
/// update every second interrupt at the DSP frame rate (`2 / F / -ln F17` = 0.2426 s).
/// VERIFIED: `F17 = 0x3F7FF38E` and the 1-update-per-pass trajectory = oracle F0 of voice 17
/// (stroke 523), `tests::key_up_record_equals_the_oracle_trajectory`, max error 1 LSB.
pub(super) fn key_up_decay_tau_s() -> f32 {
    let f17 = f64::from(f32::from_bits(RELEASE_SAMPLE_RATE));
    (2.0 / crate::organ::FIRMWARE_FRAME_RATE_HZ / -f17.ln()) as f32
}

/// The record the SMP voice-start handler leaves for a main voice (before update 1).
pub(super) fn initial_record(level: f32, rate: f32) -> SlewRecord {
    SlewRecord {
        ramp: RAMP,
        ramp_step: 0.0,
        ramp_floor: 0.0,
        ramp_ceil: RAMP_CEIL,
        x_target: 0.0,
        y_target: 0.0,
        x_coef: 1.0,
        y_decay: 1.0,
        x_state: level,
        y_state: 1.0,
        x_rate: rate,
        y_rate: 1.0,
        gain_l_mul: GAIN_MUL,
        gain_r_mul: GAIN_MUL,
    }
}

/// The slew of one voice: record, update clock and the two current gains.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct SlewVoice {
    rec: Option<SlewRecord>,
    /// Record updates per output sample: `frame_rate / 2 / sample_rate`.
    step: f64,
    acc: f64,
    left: f32,
    right: f32,
    released: bool,
    /// A key-up sample record: it decays by its own rate and ends without a note-off write.
    finite: bool,
}

impl SlewVoice {
    /// Starts a slew from the level word (`level_q23 / 2^23`) and the rate word of a note-on.
    /// The first update runs on the first sample (the oracle's first row is the handler's).
    pub(super) fn start(level: f32, rate: f32, sample_rate: f32) -> Self {
        let rec = initial_record(level, rate);
        let mut v = Self {
            rec: Some(rec),
            step: crate::organ::FIRMWARE_FRAME_RATE_HZ / 2.0 / f64::from(sample_rate),
            acc: 1.0,
            left: 0.0,
            right: 0.0,
            released: false,
            finite: false,
        };
        v.advance();
        v
    }

    /// Starts the slew of a key-up sample record (no note-off write follows; it fades by its
    /// own rate `F17`).
    pub(super) fn start_release_sample(velocity: u8, sample_rate: f32) -> Self {
        let mut v = Self::start(
            release_sample_level(velocity),
            f32::from_bits(RELEASE_SAMPLE_RATE),
            sample_rate,
        );
        v.finite = true;
        v
    }

    /// The note-off write of the ARM (applied once; later calls do nothing).
    pub(super) fn release(&mut self) {
        if self.released || self.finite {
            return;
        }
        if let Some(r) = self.rec.as_mut() {
            r.x_target = 0.0;
            r.y_target = 0.0;
            r.y_decay = 1.0;
            r.y_state = f32::from_bits(RELEASE_Y_STATE);
            self.released = true;
        }
    }

    pub(super) fn is_active(&self) -> bool {
        self.rec.is_some()
    }

    pub(super) fn is_done(&self) -> bool {
        (self.released || self.finite) && self.left.max(self.right) < DONE_GAIN
    }

    fn advance(&mut self) {
        if let Some(r) = self.rec.as_mut() {
            let g = r.step();
            self.left = g.left as f32 / Q31;
            self.right = g.right as f32 / Q31;
        }
    }

    /// Gains `(left, right)` for the current output sample, then moves the update clock.
    pub(super) fn tick(&mut self) -> (f32, f32) {
        self.acc += self.step;
        while self.acc >= 1.0 {
            self.acc -= 1.0;
            self.advance();
        }
        (self.left, self.right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(level word, rate word, left gain after update n = 1..=8)` of the oracle replay of a
    /// Velvet Grand note-on (note 60), velocities 20, 64, 100 and 127 (`sharc-ref`, SMP image,
    /// `docs/research/smp-slew.md` section 6): `F0` read after every second interrupt.
    const NOTE_ON: [(u32, u32, [i32; 8]); 4] = [
        (
            0x3D5C4D00,
            0x3F7FFE8A,
            [
                25265292, 25264729, 25264165, 25263602, 25263038, 25262475, 25261911, 25261348,
            ],
        ),
        (
            0x3EE411F0,
            0x3F7FFF42,
            [
                209252830, 209250450, 209248070, 209245690, 209243310, 209240930, 209238550,
                209236170,
            ],
        ),
        (
            0x3E66B858,
            0x3F7FFEC6,
            [
                105841323, 105839342, 105837361, 105835380, 105833399, 105831418, 105829437,
                105827456,
            ],
        ),
        (
            0x3F28E5A2,
            0x3F7FFF66,
            [
                309924507, 309921651, 309918795, 309915939, 309913083, 309910227, 309907371,
                309904515,
            ],
        ),
    ];

    /// `(decay field, semitones above the root, firmware rate word)`: Velvet Grand note 60 at
    /// velocity 20 / 64 / 100 / 127 (the oracle words of `NOTE_ON`), note 57 (1 semitone above
    /// its root) at 30 / 64 / 100, Felt Upright note 57 at velocity 100 and note 72 at 30
    /// (`tools/arm_smp_slew_init.py rates`).
    #[test]
    fn rate_law_matches_the_firmware_words() {
        let d_word = |d: u32| 0x3F80_0000u32 - d;
        for (decay, semi, d) in [
            (-187i16, 0.0f32, 374u32),
            (-157, 0.0, 314),
            (-95, 0.0, 190),
            (-77, 0.0, 154),
            (-168, 1.0, 356),
            (-144, 1.0, 306),
            (-90, 1.0, 190),
            (-159, 0.0, 318),
            (-233, 0.0, 466),
        ] {
            assert_eq!(
                rate_from_record(decay, semi).to_bits(),
                d_word(d),
                "decay {decay} semitones {semi}"
            );
        }
        for (decay, word) in [
            (-187i16, 0x3F7F_FE8Au32),
            (-157, 0x3F7F_FEC6),
            (-95, 0x3F7F_FF42),
        ] {
            assert_eq!(rate_from_record(decay, 0.0).to_bits(), word);
        }
    }

    #[test]
    fn rate_is_faster_for_higher_pitch_and_zero_without_decay() {
        assert!(rate_from_record(-100, 12.0) < rate_from_record(-100, 0.0));
        assert_eq!(rate_from_record(0, 5.0), 1.0);
        assert_eq!(rate_from_record(40, 0.0), 1.0);
    }

    /// Oracle (`sharc-ref`, SMP image) replay of the real ARM note-on messages of Felt Upright,
    /// note 57, velocity 100 (`arm_smp_slew_init.py capture` / `replay`): level word `0x3EF74664`,
    /// rate word `0x3F7FFEC2` (`d = 318`, record field `-159`), left gain `F0` after updates 1..5 (the first oracle row, 226875779, is the handler's own
    /// update 0).
    /// The rate word is the one `rate_from_record` derives from the stroke record.
    #[test]
    fn felt_upright_note_on_gains_follow_the_oracle() {
        let rate = rate_from_record(-159, 0.0);
        assert_eq!(rate.to_bits(), 0x3F7F_FEC2);
        let mut r = initial_record(f32::from_bits(0x3EF7_4664), rate);
        let want = [
            226_871_467i32,
            226_867_156,
            226_862_844,
            226_858_532,
            226_854_220,
        ];
        for (n, w) in want.into_iter().enumerate() {
            let got = r.step().left;
            assert!((got - w).abs() <= 1, "update {}: {got} vs {w}", n + 1);
        }
    }

    #[test]
    fn note_on_gains() {
        for (level, rate, want) in NOTE_ON {
            let mut r = initial_record(f32::from_bits(level), f32::from_bits(rate));
            for (n, &w) in want.iter().enumerate() {
                assert_eq!(r.step().left, w, "level {level:#x} update {}", n + 1);
            }
        }
    }

    /// Oracle: a held main voice (Velvet Grand, velocity 100) after the note-off write
    /// (`F16 = 0x3F7FBB6E` poked, `sharc-ref`): `F0` at the next five updates.
    #[test]
    fn release_gains() {
        let mut r = initial_record(f32::from_bits(0x3EE4_0E9E), f32::from_bits(0x3F7F_FF42));
        r.x_coef = f32::from_bits(RELEASE_Y_STATE); // `F13` after the first update past the write
        r.y_state = f32::from_bits(RELEASE_Y_STATE);
        let want = [
            0x0C75_6C32,
            0x0C72_0C9B,
            0x0C6E_ADF3,
            0x0C6B_5038,
            0x0C67_F36C,
        ];
        // One of the five is +1 LSB in the oracle (`q` one above the model: the rounding of the
        // 40-bit product by `FIX`, open, `docs/research/smp-slew.md` section 6).
        let mut exact = 0;
        for (n, w) in want.into_iter().enumerate() {
            let got = r.step().left;
            assert!(
                (got - w).abs() <= 1,
                "release update {}: {got} vs {w}",
                n + 1
            );
            exact += i32::from(got == w);
        }
        assert_eq!(exact, 4);
    }

    #[test]
    fn hold_gain_is_a_fifth_of_the_level_and_release_decays() {
        let level = 0.4454;
        let mut v = SlewVoice::start(level, f32::from_bits(0x3F7F_FF42), 48_000.0);
        let (l, r) = v.tick();
        assert!((l - 0.21875 * level).abs() < 1e-3 && l == r);
        for _ in 0..4800 {
            v.tick();
        }
        let hold = v.tick().0;
        v.release();
        v.release();
        for _ in 0..4800 {
            v.tick();
        }
        let after = v.tick().0;
        assert!(after < 0.5 * hold, "release must decay: {hold} -> {after}");
        assert!(!v.is_done() || after < DONE_GAIN);
    }

    /// Oracle trajectory of the key-up record (Astoria Grand, note off of a held C4, voice 17, stroke
    /// 523, `tools/smp_voice_replay.py` replay of `SMP0.script`, record `0x93CBE`, `F0` read after
    /// each pass; one record update per pass; `docs/research/piano-release-voice.md` s3). Row =
    /// `(update number n, F0 = F1, F15 word)`; n = 5 is the first row read, `F15 = 0x3F7FFFFE * F17^n`.
    const KEY_UP_ORACLE: [(usize, i32, u32); 6] = [
        (5, 0x1BF9_31EC, 0x3F7F_C1C8),
        (6, 0x1BF7_D5BC, 0x3F7F_B559),
        (9, 0x1BF3_C19F, 0x3F7F_900E),
        (105, 0x1B72_7418, 0x3F7A_F1DC),
        (235, 0x1AC7_124E, 0x3F74_D2F0),
        (384, 0x1A07_C6A3, 0x3F6D_FDF3),
    ];

    #[test]
    fn key_up_record_equals_the_oracle_trajectory() {
        let mut rec = initial_record(
            release_sample_level(100),
            f32::from_bits(RELEASE_SAMPLE_RATE),
        );
        let (mut n, mut left) = (0, 0);
        for (want_n, want_left, f15) in KEY_UP_ORACLE {
            while n < want_n {
                left = rec.step().left;
                n += 1;
            }
            assert_eq!(rec.x_state.to_bits(), f15, "F15 after {n} updates");
            // The oracle reads 1 LSB high in tie cases of the multiplier (smp-slew.md s1).
            assert!(
                (left - want_left).abs() <= 1,
                "F0 after {n} updates: {left}"
            );
        }
    }

    #[test]
    fn key_up_tau_is_0_2426_s() {
        assert!(
            (key_up_decay_tau_s() - 0.2426).abs() < 1e-3,
            "{}",
            key_up_decay_tau_s()
        );
    }

    #[test]
    fn release_sample_level_words_and_law() {
        assert_eq!(release_sample_level(20).to_bits(), RELEASE_SAMPLE_LEVEL);
        assert_eq!(release_sample_level(100).to_bits(), RELEASE_SAMPLE_LEVEL);
        assert_eq!(
            release_sample_level(127).to_bits(),
            RELEASE_SAMPLE_LEVEL_127
        );
        let mid = release_sample_level(114);
        assert!(mid < 1.0 && mid > 0.922);
    }

    #[test]
    fn release_sample_slew_decays_by_its_rate_and_ignores_the_release_write() {
        let mut v = SlewVoice::start_release_sample(64, 48_000.0);
        let (g0, _) = v.tick();
        assert!((g0 - 0.21875).abs() < 1e-3, "{g0}");
        v.release(); // must not switch to the 44 ms note-off decay
                     // One time constant (0.24 s) later the gain is about 1/e of the start.
        for _ in 0..(0.243 * 48_000.0) as usize {
            v.tick();
        }
        let g = v.tick().0;
        assert!((g / g0 - (-1.0f32).exp()).abs() < 0.05, "ratio {}", g / g0);
        for _ in 0..48_000 * 12 {
            v.tick();
        }
        assert!(v.is_done());
    }

    #[test]
    fn done_after_a_long_release_and_inactive_default() {
        assert!(!SlewVoice::default().is_active());
        let mut v = SlewVoice::start(0.5, f32::from_bits(0x3F7F_FF42), 48_000.0);
        v.release();
        for _ in 0..48_000 * 40 {
            v.tick();
        }
        assert!(v.is_done());
    }
}
