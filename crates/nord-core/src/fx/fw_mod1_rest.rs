//! Firmware Mod 1 stages RM (`kFx1Type` 4, 5), A-WAH (6, 7), WAH (8, 9) and PUMP (10, 11), behind
//! `FirmwareFx::mod1` (opt-in, D22).
//!
//! The routines ([`super::fw_rm`] `0x1239D0`, [`super::fw_awah`] `0x123E9B`, [`super::fw_wah`]
//! `0x123DD2`, [`super::fw_pump`] `0x122D2C`) are VERIFIED against the oracle. The piano unit
//! calls them on **one block of two stereo frames** (the boot words `c[-16]`, `c[4]`, `c[7]`,
//! `c[-8]` are 1), so the stage collects 2 frames and releases them one call later: **latency 2
//! frames (46 us at 44.1 kHz)**. The phase increment words are per call and are scaled by
//! `FIRMWARE_FRAME_RATE_HZ / fs`, so a carrier or an LFO keeps its frequency in Hz; the one-pole
//! coefficients of the filters stay those of 43.4 kHz (ASSUMED, a few percent in cutoff).
//!
//! Words (`docs/research/fx-mod-types.md` s7): the instance row is the image under the boot words
//! ([`super::fw_mod1_data`]), then the words the type switch writes, then the 11 samples of the
//! `kFx1Amt` / `kFx1RateClkDiv` laws (VERIFIED at the samples, EMPIRICAL between them). Self
//! pointers of the row are relocated to the context of the port.

use super::fw_chorus::CTX_I1;
use super::fw_mod1_data as d;
use super::fw_mod1_dense as dense;
use super::fw_mod2_data::{Law, SAMPLES};
use super::fw_mod2_run::{process, Mod2State, Routine};
use super::fw_pump_clock::{multiplier, ClockWord, CLOCK_ADDRESS, CLOCK_WORD};

/// PUMP position pointer `c[-29]` and phase multiplier `c[-28]` (rewritten by the ARM under MST CLK).
const PUMP_POINTER: i32 = -29;
const PUMP_MULTIPLIER: i32 = -28;

/// One wired type: routine, instance row (and its `I1`), type words, laws, phase words.
struct Spec {
    routine: Routine,
    base: u32,
    row: &'static [(i32, u32)],
    words: &'static [(i32, u32)],
    amt: &'static [Law],
    rate: &'static [Law],
    /// Integer words that are phase increments per call (scaled by `F / fs`).
    phase: &'static [i32],
}

fn spec(ty: u8) -> Option<Spec> {
    let (routine, base, row, words, amt, rate, phase): (_, _, _, _, _, _, &[i32]) = match ty {
        4 => (
            Routine::Rm,
            0xB04B4,
            d::ROW_RM,
            d::WORDS_4,
            d::AMT_4,
            d::RATE_4,
            &[-32],
        ),
        5 => (
            Routine::Rm,
            0xB04B4,
            d::ROW_RM,
            d::WORDS_5,
            d::AMT_5,
            d::RATE_5,
            &[-32],
        ),
        6 => (
            Routine::Awah,
            0xC1946,
            d::ROW_AWAH,
            d::WORDS_6,
            d::AMT_6,
            d::RATE_6,
            &[],
        ),
        7 => (
            Routine::Awah,
            0xC1946,
            d::ROW_AWAH,
            d::WORDS_7,
            d::AMT_7,
            d::RATE_7,
            &[],
        ),
        8 => (
            Routine::Wah,
            0xC1884,
            d::ROW_WAH,
            d::WORDS_8,
            d::AMT_8,
            d::RATE_8,
            &[-32],
        ),
        9 => (
            Routine::Wah,
            0xC1884,
            d::ROW_WAH,
            d::WORDS_9,
            d::AMT_9,
            d::RATE_9,
            &[-32],
        ),
        10 => (
            Routine::Pump,
            0xC16CA,
            d::ROW_PUMP,
            d::WORDS_10,
            d::AMT_10,
            d::RATE_10,
            &[-32],
        ),
        11 => (
            Routine::Pump,
            0xC16CA,
            d::ROW_PUMP,
            d::WORDS_11,
            d::AMT_11,
            d::RATE_11,
            &[-32],
        ),
        _ => return None,
    };
    Some(Spec {
        routine,
        base,
        row,
        words,
        amt,
        rate,
        phase,
    })
}

fn index(offset: i32) -> usize {
    (offset + 0x20) as usize
}

/// A law at `v` (`0..=127`): linear between the neighbouring samples, exact at them.
pub(super) fn eval(law: &Law, v: u32) -> f32 {
    let v = v.min(127);
    let k = SAMPLES.iter().rposition(|&s| s <= v).unwrap_or(0).min(9);
    let (x0, x1) = (SAMPLES[k], SAMPLES[k + 1]);
    let t = (v - x0) as f32 / (x1 - x0) as f32;
    let (a, b) = (law.words[k], law.words[k + 1]);
    if law.float {
        let (a, b) = (f32::from_bits(a), f32::from_bits(b));
        a + (b - a) * t
    } else {
        let (a, b) = (a as i32 as f32, b as i32 as f32);
        a + (b - a) * t
    }
}

/// The word of `x` for a law (a float word or an integer).
pub(super) fn to_word(law: &Law, x: f32) -> u32 {
    if law.float {
        x.to_bits()
    } else {
        x.round() as i32 as u32
    }
}

/// Firmware routine of a Mod 1 type plus the 2-frame buffers.
#[derive(Debug, Clone)]
pub(super) struct FwMod1Rest {
    ty: u8,
    st: Mod2State,
    phase_scale: f64,
    input: [[f32; 2]; 2],
    output: [[f32; 2]; 2],
    pos: usize,
    /// PUMP only: Master Clock division period in engine frames (`None`: free run).
    clock_period: Option<f64>,
    /// Phase multiplier `N` written to `c[-28]` under the clock (`fw_pump_clock`).
    clock_n: u32,
    clock: ClockWord,
}

impl FwMod1Rest {
    /// Stage for `kFx1Type` `ty` (4..=11), `kFx1RateClkDiv` and `kFx1Amt` in `0..=127`, at `fs`;
    /// `None` for the other types.
    pub(super) fn new(ty: u8, rate: u32, amount: u32, fs: f32) -> Option<Self> {
        let sp = spec(ty)?;
        let mut st = Mod2State::new();
        for &(o, w) in sp.row {
            // Self pointers of the image / boot words address the row of the unit: relocate them.
            let lo = sp.base.wrapping_sub(0x20);
            st.ctx[index(o)] = if (lo..=sp.base + 0x2A).contains(&w) {
                CTX_I1 + w - sp.base
            } else {
                w
            };
        }
        for &(o, w) in sp.words {
            st.ctx[index(o)] = if (sp.base - 0x20..=sp.base + 0x2A).contains(&w) {
                CTX_I1 + w - sp.base
            } else {
                w
            };
        }
        let mut me = Self {
            ty,
            st,
            phase_scale: crate::organ::FIRMWARE_FRAME_RATE_HZ / f64::from(fs),
            input: [[0.0; 2]; 2],
            output: [[0.0; 2]; 2],
            pos: 0,
            clock_period: None,
            clock_n: 1,
            clock: ClockWord::default(),
        };
        me.set_params(rate, amount);
        Some(me)
    }

    /// Parks the stage: the type no longer matches any request, so the next use re-initialises
    /// it (`ty` 255 has no `spec`).
    pub(super) fn retire(&mut self) {
        self.ty = u8::MAX;
        self.clock_period = None;
    }

    /// The `kFx1Type` this stage was built for.
    pub(super) fn ty(&self) -> u8 {
        self.ty
    }

    /// Follows the Master Clock (PUMP types 10, 11 only): `Some((period, beats))` is the division
    /// of `beats` beats lasting `period` engine frames; it points `c[-29]` at the clock word and
    /// sets `c[-28]` to `N = 32 / beats` like the ARM (`fw_pump_clock`). `None` is free run (own
    /// phase, `N` = 1). Other types ignore it.
    pub(super) fn set_clock_period(&mut self, clock: Option<(f64, f32)>) {
        let clock = clock.filter(|_| matches!(self.ty, 10 | 11));
        let was_synced = self.clock_period.is_some();
        if clock.is_some() != was_synced {
            self.clock.restart();
        }
        self.clock_period = clock.map(|c| c.0);
        self.clock_n = clock.map_or(1, |c| multiplier(c.1));
        // Free run that was never synced keeps its boot words untouched.
        if clock.is_some() || was_synced {
            self.st.ctx[index(PUMP_POINTER)] = if clock.is_some() {
                CLOCK_ADDRESS
            } else {
                CTX_I1 - 31
            };
            self.st.ctx[index(PUMP_MULTIPLIER)] = self.clock_n;
        }
    }

    /// Writes the targets of `kFx1RateClkDiv` and `kFx1Amt`.
    pub(super) fn set_params(&mut self, rate: u32, amount: u32) {
        let Some(sp) = spec(self.ty) else { return };
        for (laws, v, is_rate) in [(sp.amt, amount, false), (sp.rate, rate, true)] {
            for law in laws {
                // The RM phase increment and the A-WAH rate are not linear between the 11 samples:
                // they have 44 knots (`fw_mod1_dense`, VERIFIED at the knots, D100).
                let mut x = match is_rate.then(|| dense::dense(self.ty, law.off)).flatten() {
                    Some(w) => dense::eval(w, law.float, v),
                    None => eval(law, v),
                };
                if sp.phase.contains(&law.off) {
                    x = (f64::from(x) * self.phase_scale).round() as f32;
                }
                self.st.ctx[index(law.off)] = to_word(law, x);
            }
        }
    }

    /// Pushes one frame and returns the frame of the previous block at the same position.
    #[inline]
    pub(super) fn process_frame(&mut self, frame: [f32; 2]) -> [f32; 2] {
        let out = self.output[self.pos];
        self.input[self.pos] = frame;
        self.pos += 1;
        if self.pos == 2 {
            self.pos = 0;
            let routine = spec(self.ty).map_or(Routine::Rm, |s| s.routine);
            if let Some(period) = self.clock_period {
                // The clock word wraps every `N` divisions; one call is two frames.
                self.st.ctx[index(CLOCK_WORD)] =
                    self.clock.advance(2.0, period * f64::from(self.clock_n));
            }
            let mut block: [f32; 4] = std::array::from_fn(|i| self.input[i / 2][i % 2]);
            process(routine, &mut self.st, &mut block);
            self.output = std::array::from_fn(|k| [block[2 * k], block[2 * k + 1]]);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(ty: u8, rate: u32, amount: u32, fs: f32, n: usize) -> Vec<[f32; 2]> {
        let mut st = FwMod1Rest::new(ty, rate, amount, fs).unwrap();
        (0..n)
            .map(|i| st.process_frame([((i as f32) * 0.05).sin(), ((i as f32) * 0.031).cos()]))
            .collect()
    }

    #[test]
    fn every_type_runs_finite_with_a_two_frame_latency() {
        for ty in 4..=11 {
            for (rate, amount) in [(0, 0), (64, 64), (127, 127), (30, 100)] {
                let out = run(ty, rate, amount, 48_000.0, 6000);
                assert!(
                    out.iter()
                        .all(|f| f.iter().all(|x| x.is_finite() && x.abs() < 16.0)),
                    "type {ty} rate {rate} amount {amount}"
                );
                assert!(out[..2].iter().all(|f| *f == [0.0, 0.0]), "latency");
                assert!(
                    out.iter().skip(2).any(|f| f[0] != 0.0),
                    "type {ty} rate {rate} amount {amount} is silent"
                );
            }
        }
        for ty in [0, 1, 2, 3, 12] {
            assert!(FwMod1Rest::new(ty, 0, 0, 48_000.0).is_none(), "type {ty}");
        }
    }

    #[test]
    fn the_laws_hit_the_captured_samples() {
        for ty in 4..=11 {
            let sp = spec(ty).unwrap();
            for law in sp.amt.iter().chain(sp.rate) {
                for (k, &v) in SAMPLES.iter().enumerate() {
                    assert_eq!(to_word(law, eval(law, v)), law.words[k], "type {ty} v {v}");
                }
            }
        }
    }

    #[test]
    fn the_dense_rate_words_follow_the_arm_between_the_samples() {
        // ARM words at v = 120 (`fx-mod1-dense.csv`): RM phase increment 0xaa84a00, A-WAH 0x3c...
        // The 11-sample law was 6.4 % of the range off there.
        let rm = FwMod1Rest::new(4, 120, 64, crate::organ::FIRMWARE_FRAME_RATE_HZ as f32).unwrap();
        assert_eq!(rm.st.ctx[index(-32)], 0x0aa8_4a00);
        let sample_only = eval(&spec(4).unwrap().rate[0], 120);
        assert!((sample_only - 0x0aa8_4a00_u32 as f32).abs() > 1.0e6);
        for ty in [4, 5] {
            let at = |v| {
                FwMod1Rest::new(ty, v, 64, crate::organ::FIRMWARE_FRAME_RATE_HZ as f32)
                    .unwrap()
                    .st
                    .ctx[index(-32)]
            };
            assert_eq!(at(127), 0x0fb6_3b00, "type {ty}");
            assert_eq!(at(2), 0x0003_ed00, "type {ty}");
        }
    }

    #[test]
    fn the_boot_words_give_the_self_pointers() {
        // RM, WAH and PUMP read the phase through `c[-29]`; the boot word is the address of
        // `c[-31]` in the row, relocated to the context of the port.
        for ty in [4, 8, 10] {
            let st = FwMod1Rest::new(ty, 64, 64, 48_000.0).unwrap();
            assert_eq!(st.st.ctx[index(-29)], CTX_I1 - 31, "type {ty}");
        }
    }

    #[test]
    fn phase_increments_scale_with_the_engine_rate() {
        for ty in [4, 8, 10] {
            let at = |fs: f32| FwMod1Rest::new(ty, 90, 64, fs).unwrap().st.ctx[index(-32)] as f64;
            let ratio = at(86_804.69) / at(43_402.34);
            assert!((ratio - 0.5).abs() < 1.0e-3, "type {ty}: {ratio}");
        }
    }

    #[test]
    fn rate_and_amount_change_the_output() {
        for ty in 4..=11 {
            assert_ne!(
                run(ty, 20, 90, 48_000.0, 3000),
                run(ty, 100, 90, 48_000.0, 3000),
                "rate {ty}"
            );
            assert_ne!(
                run(ty, 60, 10, 48_000.0, 3000),
                run(ty, 60, 120, 48_000.0, 3000),
                "amount {ty}"
            );
        }
    }

    // PUMP sync tests (counter, clock word, `c[0x2A]`): `fw_mod1_rest/sync_tests.rs`.
}

#[cfg(test)]
mod sync_tests;
