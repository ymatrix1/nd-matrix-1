//! Rate matching of the per-frame firmware FX stages (default on since D176,
//! `LayerFxChain::set_fx_rate_match`).
//!
//! The firmware runs its FX routines once per DSP frame at `F = 43,402.34375 Hz`
//! (`docs/research/dsp-frame-rate.md`), and the Amp drive and cabinet words (`fw_amp_*`) are
//! per-frame coefficients and taps. The engine renders at the host rate (48 kHz) and calls the
//! same stage once per host frame, so every response feature sits `fs / F = 1.106` times higher
//! in Hz than on the hardware (`docs/research/brightness-downstream-oracle.md`: +0.5 to +2.2 dB in
//! 3..8 kHz, up to +3.5 dB in 8..16 kHz and 3 to 7 dB in single third octaves of the Small, Twin
//! and JC cabinets).
//!
//! [`RateMatch`] runs a stage at its own frame rate: the host stream is resampled down to `F`
//! with a windowed sinc, the stage runs once per `F` frame, and its output is resampled back up.
//! Both kernels are Blackman windowed sincs with 16 zero crossings per side (cut-off `F / 2`),
//! tabulated at 256 sub-sample phases and linearly interpolated.
//!
//! * Latency: `LATENCY_FRAMES` host frames of the two kernels (about 0.8 ms at 48 kHz) plus the
//!   stage's own latency (4 frames at `F` for the Amp stage). The first outputs are silence.
//! * Real time: the kernel tables are built in [`RateMatch::new`] and shared (`Arc`); the
//!   per-frame path touches fixed rings only, no allocation, lock or I/O.
//! * Limits: the host rate must lie in about 44.1 to 169 kHz (`2 * hd <= RING` is asserted in
//!   [`RateMatch::new`]; below the frame rate the up kernel is cut above the host Nyquist and
//!   would alias). The tables are built for the rate given to `new`; a chain whose sample rate
//!   changes afterwards must call `set_fx_rate_match` again.
//! * Provenance: the rate is VERIFIED (`dsp-frame-rate.md`); the kernel shape (length, window,
//!   cut-off) is ASSUMED (chosen for passband flatness, checked by the tests below: gain within
//!   0.05 dB to 16 kHz and 0.3 dB for a per-frame one-pole against its analytic response).

use std::sync::Arc;

use crate::organ::FIRMWARE_FRAME_RATE_HZ;

/// Zero crossings per side of the (low-pass) kernels, measured in stage frames.
const ZC: usize = 16;
/// Sub-sample phases of the tables (one extra row closes the interval).
const PHASES: usize = 256;
/// Ring length in frames (power of two, larger than the widest kernel).
const RING: usize = 128;

/// Kernel tables of one rate pair.
#[derive(Debug)]
struct Tables {
    /// Input frames per stage frame, `fs / F`.
    ratio: f64,
    /// Half width of the down kernel in host frames.
    hd: usize,
    /// `(PHASES + 1) x 2 hd` taps for host to stage.
    down: Vec<f32>,
    /// `(PHASES + 1) x 2 ZC` taps for stage to host.
    up: Vec<f32>,
}

fn blackman(x: f64) -> f64 {
    if x.abs() >= 1.0 {
        0.0
    } else {
        0.42 + 0.5 * (std::f64::consts::PI * x).cos() + 0.08 * (std::f64::consts::TAU * x).cos()
    }
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
    }
}

impl Tables {
    fn new(fs: f64) -> Self {
        let ratio = fs / FIRMWARE_FRAME_RATE_HZ;
        let hd = (ZC as f64 * ratio.max(1.0)).ceil() as usize + 1;
        assert!(
            2 * hd <= RING,
            "fx rate match: host rate {fs} Hz needs a {}-frame kernel, the ring holds {RING}",
            2 * hd
        );
        let mut down = vec![0.0f32; (PHASES + 1) * 2 * hd];
        for p in 0..=PHASES {
            let frac = p as f64 / PHASES as f64;
            for i in 0..2 * hd {
                // tap i reads host frame floor(c) - hd + 1 + i, so t = c - m = frac + hd - 1 - i
                let t = frac + hd as f64 - 1.0 - i as f64;
                let k = sinc(t / ratio) / ratio * blackman(t / (ZC as f64 * ratio));
                down[p * 2 * hd + i] = k as f32;
            }
        }
        let mut up = vec![0.0f32; (PHASES + 1) * 2 * ZC];
        for p in 0..=PHASES {
            let frac = p as f64 / PHASES as f64;
            for i in 0..2 * ZC {
                let t = frac + ZC as f64 - 1.0 - i as f64;
                up[p * 2 * ZC + i] = (sinc(t) * blackman(t / ZC as f64)) as f32;
            }
        }
        Self {
            ratio,
            hd,
            down,
            up,
        }
    }
}

/// Runs a per-frame stage at the firmware frame rate inside a host-rate chain.
#[derive(Debug, Clone)]
pub struct RateMatch {
    t: Arc<Tables>,
    xin: [[f32; 2]; RING],
    yout: [[f32; 2]; RING],
    /// Host frames pushed.
    n_in: u64,
    /// Stage frames produced.
    n_y: u64,
    /// Host frames emitted.
    n_out: u64,
}

/// Interpolated row of a `(PHASES + 1)`-row table at fractional phase `frac`.
#[inline]
fn row(table: &[f32], width: usize, frac: f64, i: usize) -> f32 {
    let x = frac * PHASES as f64;
    let p = (x as usize).min(PHASES - 1);
    let w = (x - p as f64) as f32;
    let a = table[p * width + i];
    let b = table[(p + 1) * width + i];
    a + (b - a) * w
}

impl RateMatch {
    /// Whether host rate `fs` is inside the supported range: at least the frame rate (below it the
    /// up kernel would alias) and a down kernel that fits the ring (about 170 kHz).
    pub fn supports(fs: f32) -> bool {
        let ratio = f64::from(fs) / FIRMWARE_FRAME_RATE_HZ;
        let hd = (ZC as f64 * ratio.max(1.0)).ceil() as usize + 1;
        fs >= 44_000.0 && 2 * hd <= RING
    }

    /// Builds the tables for host rate `fs` (allocates; call outside the audio thread).
    pub fn new(fs: f32) -> Self {
        Self {
            t: Arc::new(Tables::new(f64::from(fs))),
            xin: [[0.0; 2]; RING],
            yout: [[0.0; 2]; RING],
            n_in: 0,
            n_y: 0,
            n_out: 0,
        }
    }

    /// Host frames of delay of the two kernels (excluding the stage's own latency).
    pub fn latency_frames(&self) -> usize {
        ((self.t.hd as f64 + ZC as f64 * self.t.ratio) as usize).max(1)
    }

    /// One host frame in, one out; `stage` is called once per stage frame (zero, one or two
    /// times per host frame). The first `latency_frames` outputs are silence.
    pub fn process_frame(
        &mut self,
        frame: [f32; 2],
        mut stage: impl FnMut([f32; 2]) -> [f32; 2],
    ) -> [f32; 2] {
        let Self {
            t,
            xin,
            yout,
            n_in,
            n_y,
            n_out,
        } = self;
        xin[(*n_in as usize) % RING] = frame;
        *n_in += 1;
        let hd = t.hd;
        // host to stage: frame k is centred on host position c = k * ratio
        loop {
            let c = *n_y as f64 * t.ratio;
            let base = c.floor() as u64;
            if base + hd as u64 >= *n_in {
                break;
            }
            let frac = c - base as f64;
            let mut acc = [0.0f32; 2];
            for i in 0..2 * hd {
                let m = (base + 1 + i as u64).wrapping_sub(hd as u64);
                if base + 1 + (i as u64) < hd as u64 {
                    continue; // before the first frame: zero history
                }
                let k = row(&t.down, 2 * hd, frac, i);
                let x = xin[(m as usize) % RING];
                acc[0] += k * x[0];
                acc[1] += k * x[1];
            }
            yout[(*n_y as usize) % RING] = stage(acc);
            *n_y += 1;
        }
        // stage to host: output n sits at stage position u = n / ratio
        let u = *n_out as f64 / t.ratio;
        let q0 = u.floor() as u64;
        if q0 + ZC as u64 >= *n_y {
            return [0.0; 2];
        }
        let frac = u - q0 as f64;
        let mut acc = [0.0f32; 2];
        for i in 0..2 * ZC {
            if q0 + 1 + (i as u64) < ZC as u64 {
                continue;
            }
            let q = q0 + 1 + i as u64 - ZC as u64;
            let k = row(&t.up, 2 * ZC, frac, i);
            let y = yout[(q as usize) % RING];
            acc[0] += k * y[0];
            acc[1] += k * y[1];
        }
        *n_out += 1;
        acc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Amplitude of a steady sine of `hz` through the matcher at `fs` with the per-frame stage.
    fn sine_gain(fs: f32, hz: f32, mut stage: impl FnMut([f32; 2]) -> [f32; 2]) -> f64 {
        let mut rm = RateMatch::new(fs);
        let n = 20_000;
        let (mut s, mut c, mut p) = (0.0f64, 0.0f64, 0.0f64);
        for i in 0..n {
            let ph = std::f64::consts::TAU * f64::from(hz) * i as f64 / f64::from(fs);
            let y = rm.process_frame([ph.sin() as f32; 2], &mut stage)[0];
            if i > 2000 {
                s += f64::from(y) * ph.sin();
                c += f64::from(y) * ph.cos();
                p += 1.0;
            }
        }
        2.0 * (s * s + c * c).sqrt() / p
    }

    /// Same measurement as [`sine_gain`] with the stage called once per host frame (no matcher).
    fn sine_gain_unmatched(fs: f32, hz: f32, mut stage: impl FnMut([f32; 2]) -> [f32; 2]) -> f64 {
        let (mut s, mut c, mut p) = (0.0f64, 0.0f64, 0.0f64);
        for i in 0..20_000 {
            let ph = std::f64::consts::TAU * f64::from(hz) * i as f64 / f64::from(fs);
            let y = stage([ph.sin() as f32; 2])[0];
            if i > 2000 {
                s += f64::from(y) * ph.sin();
                c += f64::from(y) * ph.cos();
                p += 1.0;
            }
        }
        2.0 * (s * s + c * c).sqrt() / p
    }

    #[test]
    fn supports_covers_44k_to_the_ring_limit_only() {
        for fs in [44_100.0, 48_000.0, 88_200.0, 96_000.0, 160_000.0] {
            assert!(RateMatch::supports(fs), "{fs}");
        }
        for fs in [8_000.0, 22_050.0, 43_000.0, 192_000.0] {
            assert!(!RateMatch::supports(fs), "{fs}");
        }
    }

    #[test]
    #[should_panic(expected = "ring holds")]
    fn a_host_rate_too_high_for_the_ring_is_rejected() {
        let _ = RateMatch::new(192_000.0);
    }

    #[test]
    fn identity_stage_is_flat_to_16_khz_within_0_05_db() {
        for hz in [100.0, 1000.0, 5000.0, 10_000.0, 16_000.0] {
            let g = sine_gain(48_000.0, hz, |x| x);
            let db = 20.0 * g.log10();
            assert!(db.abs() < 0.05, "{hz} Hz: {db} dB");
        }
    }

    #[test]
    fn a_per_frame_one_pole_gets_its_response_in_hz_of_the_frame_rate() {
        let a = 0.3f32;
        let f_hz = FIRMWARE_FRAME_RATE_HZ;
        for hz in [3000.0f32, 6000.0, 12_000.0] {
            let mut y = [0.0f32; 2];
            let g = sine_gain(48_000.0, hz, |x| {
                y = [y[0] + a * (x[0] - y[0]), y[1] + a * (x[1] - y[1])];
                y
            });
            let w = std::f64::consts::TAU * f64::from(hz) / f_hz;
            let (re, im) = (
                1.0 - f64::from(1.0 - a) * w.cos(),
                f64::from(1.0 - a) * w.sin(),
            );
            let want = f64::from(a) / (re * re + im * im).sqrt();
            let err = 20.0 * (g / want).log10();
            assert!(err.abs() < 0.3, "{hz} Hz: {err} dB (got {g}, want {want})");
            // the unmatched stage at 48 kHz is off by the 1.106 frequency stretch
            let mut z = [0.0f32; 2];
            let z_out = sine_gain_unmatched(48_000.0, hz, |x| {
                z = [z[0] + a * (x[0] - z[0]), z[1] + a * (x[1] - z[1])];
                z
            });
            assert!(
                (20.0 * (z_out / want).log10()).abs() > err.abs(),
                "{hz} Hz: matcher must be closer than the unmatched stage"
            );
        }
    }

    #[test]
    fn silence_stays_silent_the_first_outputs_are_zero_and_latency_is_bounded() {
        let mut rm = RateMatch::new(48_000.0);
        let lat = rm.latency_frames();
        assert!((30..=50).contains(&lat), "latency {lat}");
        for _ in 0..1000 {
            assert_eq!(rm.process_frame([0.0; 2], |x| x), [0.0; 2]);
        }
        let mut rm = RateMatch::new(48_000.0);
        let first: Vec<f32> = (0..lat - 4)
            .map(|_| rm.process_frame([1.0; 2], |x| x)[0])
            .collect();
        assert!(first.iter().all(|v| *v == 0.0), "{first:?}");
    }

    #[test]
    fn the_frame_rate_itself_gives_a_ratio_of_one_and_stays_flat() {
        let g = sine_gain(FIRMWARE_FRAME_RATE_HZ as f32, 5000.0, |x| x);
        assert!((20.0 * g.log10()).abs() < 0.05, "{g}");
    }
}
