//! Cases and measurements of `fx_engine_vs_fw`: every FX family of the layer chain rendered by
//! the engine path and by the firmware port on the same deterministic stimulus
//! (`docs/research/fx-vs-oracle.md`). The ports are the ones checked word for word against the
//! SHARC oracle (golden vectors, and the live chain for Mod 1 / Mod 2 / reverb in
//! `tools/fx_vs_oracle.py`), so engine against port is engine against firmware. Numbers only.
use crate::band_response::{band_gain_db, noise};
use nord_core::cbin::FxDecoded;
use nord_core::engine::Stage4Engine;
use nord_core::fx::{FirmwareFx, LayerFxChain};

/// DSP frame rate (Hz): no rate scaling in the ports.
/// VERIFIED: equals `FIRMWARE_FRAME_RATE_HZ` (`crates/nord-core/src/organ/lane_mix.rs`, 43,402.34375 Hz, the
/// rate the live FX oracle runs at; `fx_live_native` and `tools/fx_vs_oracle.py` use the same value).
pub const FS: f32 = 43_402.344;
/// Octave bands of the band gains (Hz).
/// ASSUMED: plain octave edges 31 .. 16,000 Hz chosen as a reporting grid, not a firmware quantity;
/// they only label where a level or response difference sits.
pub const BANDS: [(f64, f64); 9] = [
    (31.0, 63.0),
    (63.0, 125.0),
    (125.0, 250.0),
    (250.0, 500.0),
    (500.0, 1000.0),
    (1000.0, 2000.0),
    (2000.0, 4000.0),
    (4000.0, 8000.0),
    (8000.0, 16000.0),
];

/// One case: a family, a label, the decoded FX block and the port that is the firmware path.
#[derive(Clone)]
pub struct Case {
    pub family: &'static str,
    pub label: String,
    pub dec: FxDecoded,
    /// `FirmwareFx` port name (`PORT_NAMES`), `amp` for the amp drive, `""` = no port.
    pub port: &'static str,
}

fn base() -> FxDecoded {
    let mut d = FxDecoded::default();
    d.mod1_enabled = false;
    d.mod2_enabled = false;
    d.amp_enabled = false;
    d.comp_enabled = false;
    d.delay_enabled = false;
    d.reverb_enabled = false;
    d
}

/// All cases (Mod 1 and Mod 2 types at three amounts, reverb types at two amounts, compressor,
/// EQ, amp modes, delay, delay feedback effects).
pub fn cases() -> Vec<Case> {
    let mut v = Vec::new();
    let mk = |family, label: String, port, f: &dyn Fn(&mut FxDecoded)| {
        let mut dec = base();
        f(&mut dec);
        Case {
            family,
            label,
            dec,
            port,
        }
    };
    for ty in 0..12u8 {
        for (a, name) in [(0.0, "amt0"), (0.5, "amt64"), (1.0, "amt127")] {
            v.push(mk("mod1", format!("type{ty}-{name}"), "mod1", &|d| {
                (d.mod1_enabled, d.mod1_type_raw, d.mod1_rate, d.mod1_amount) = (true, ty, 0.5, a);
            }));
            v.push(mk("mod2", format!("type{ty}-{name}"), "mod2", &|d| {
                (d.mod2_enabled, d.mod2_type_raw, d.mod2_rate, d.mod2_amount) = (true, ty, 0.5, a);
            }));
        }
        for (a, name) in [(0.25, "amt32"), (1.0, "amt127")] {
            v.push(mk("reverb", format!("type{ty}-{name}"), "reverb", &|d| {
                d.reverb_enabled = true;
                d.reverb_type_raw = ty >> 1;
                d.amp_type_new = u16::from(ty & 1) << 8;
                d.reverb_amount = a;
            }));
        }
    }
    for (a, fast, name) in [
        (0.25, false, "amt32-slow"),
        (0.5, false, "amt64-slow"),
        (1.0, false, "amt127-slow"),
        (0.5, true, "amt64-fast"),
    ] {
        v.push(mk("comp", name.to_string(), "compressor", &|d| {
            (d.comp_enabled, d.comp_amount, d.comp_fast) = (true, a, fast);
        }));
    }
    for (b, m, t, name) in [
        (12.0, 0.0, 0.0, "bass+12"),
        (-12.0, 0.0, 0.0, "bass-12"),
        (0.0, 12.0, 0.0, "mid+12"),
        (0.0, -12.0, 0.0, "mid-12"),
        (0.0, 0.0, 12.0, "treble+12"),
        (0.0, 0.0, -12.0, "treble-12"),
    ] {
        v.push(mk("eq", name.to_string(), "eq", &|d| {
            (d.amp_enabled, d.eq_bass_db, d.eq_mid_db, d.eq_treble_db) = (true, b, m, t);
            d.eq_mid_freq_norm = 0.5;
        }));
    }
    for mode in 1..10u16 {
        for (drive, name) in [(0.0, "drive0"), (5.0, "drive5"), (10.0, "drive10")] {
            v.push(mk("amp", format!("mode{mode}-{name}"), "amp", &|d| {
                (d.amp_enabled, d.amp_type_new, d.amp_drive) = (true, mode, drive);
            }));
        }
    }
    for (mix, fb, name) in [
        (0.5, 0.0, "mix64-fb0"),
        (0.5, 0.5, "mix64-fb64"),
        (1.0, 0.5, "mix127-fb64"),
    ] {
        v.push(mk("delay", name.to_string(), "delay-laws", &|d| {
            (
                d.delay_enabled,
                d.delay_rate_norm,
                d.delay_mix,
                d.delay_feedback,
            ) = (true, 0.5, mix, fb);
        }));
    }
    for ty in 2..12u8 {
        v.push(mk("delayfx", format!("type{ty}"), "", &|d| {
            (
                d.delay_enabled,
                d.delay_rate_norm,
                d.delay_mix,
                d.delay_feedback,
            ) = (true, 0.5, 0.5, 0.4);
            d.delay_fb_fx_type = ty;
        }));
    }
    v
}

/// The chain of a case: `fw` turns the case's port on (every other firmware path off).
pub fn chain(case: &Case, fw: bool) -> LayerFxChain {
    let mut fx = LayerFxChain::new(FS);
    fx.firmware_fx = FirmwareFx::OFF;
    if fw {
        if case.port == "amp" {
            fx.firmware_fx.amp = true;
        } else if let Some(p) = fx.firmware_fx.port_mut(case.port) {
            *p = true;
        }
        if case.port == "mod1" {
            // PUMP (types 10, 11) has its own switch since D134; the other Mod 1 types follow `mod1`.
            fx.firmware_fx.pump = true;
        }
        if case.port == "delay-laws" {
            fx.firmware_fx.delay_time = true;
        }
    }
    Stage4Engine::apply_fx_chain(&mut fx, &case.dec);
    fx
}

/// Stimulus `kind` (`impulse`, `sweep`, `noise`, `notes`), mono, `n` frames; the second value is
/// the frame where its energy ends.
pub fn stimulus(kind: &str, n: usize) -> (Vec<f32>, usize) {
    let t = |k: usize| k as f64 / f64::from(FS);
    match kind {
        "impulse" => (
            std::iter::once(0.5)
                .chain(std::iter::repeat(0.0))
                .take(n)
                .collect(),
            1,
        ),
        "sweep" => {
            let (d, k) = (0.5f64, (16_000.0f64 / 30.0).ln());
            let x = (0..n)
                .map(|i| {
                    let s = t(i).min(d);
                    let ph = 2.0 * std::f64::consts::PI * 30.0 * d / k * ((s / d * k).exp() - 1.0);
                    if t(i) < d {
                        (0.25 * ph.sin()) as f32
                    } else {
                        0.0
                    }
                })
                .collect();
            (x, (d * f64::from(FS)) as usize)
        }
        "notes" => {
            let x = (0..n)
                .map(|i| {
                    let s = t(i);
                    let rel = if s < 0.5 {
                        1.0
                    } else {
                        (-(s - 0.5) / 0.08).exp()
                    };
                    let mut y = 0.0;
                    for (f0, t0, g) in [(220.0, 0.0, 0.12), (330.0, 0.2, 0.09)] {
                        let on = ((s - t0) / 0.005).clamp(0.0, 1.0) * f64::from(s >= t0);
                        let tone: f64 = (1..=6)
                            .map(|h| {
                                (2.0 * std::f64::consts::PI * f0 * f64::from(h) * s).sin()
                                    / f64::from(h)
                            })
                            .sum();
                        y += g * on * rel * tone;
                    }
                    y as f32
                })
                .collect();
            (x, (0.5 * f64::from(FS)) as usize)
        }
        _ => {
            let mut x = noise(n, 7, 0.1);
            let end = (0.5 * f64::from(FS)) as usize;
            x.iter_mut().skip(end).for_each(|v| *v = 0.0);
            (x, end)
        }
    }
}

/// Left output of a chain on the mono stimulus (both channels equal).
pub fn render(fx: &mut LayerFxChain, x: &[f32]) -> Vec<f32> {
    x.iter().map(|&v| fx.process_frame([v, v])[0]).collect()
}

/// RMS in dB.
pub fn rms_db(x: &[f32]) -> f64 {
    let e = x.iter().map(|&v| f64::from(v).powi(2)).sum::<f64>() / x.len().max(1) as f64;
    10.0 * e.max(1e-30).log10()
}

/// Octave band gains (dB) of `y` re `x`.
pub fn bands_db(x: &[f32], y: &[f32]) -> Vec<f64> {
    BANDS
        .iter()
        .map(|b| band_gain_db(x, y, f64::from(FS), b.0, b.1))
        .collect()
}

/// Spectral centroid (Hz) of `x` from the power spectrum.
pub fn centroid(x: &[f32]) -> f64 {
    let p = crate::band_response::power_spectrum(x);
    let bin = f64::from(FS) / (2.0 * p.len() as f64);
    let (num, den) = p
        .iter()
        .enumerate()
        .fold((0.0, 0.0), |(n, d), (k, v)| (n + k as f64 * bin * v, d + v));
    if den > 0.0 {
        num / den
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_case_table_covers_every_family_and_builds_chains() {
        let c = cases();
        for fam in [
            "mod1", "mod2", "reverb", "comp", "eq", "amp", "delay", "delayfx",
        ] {
            assert!(c.iter().any(|x| x.family == fam), "{fam}");
        }
        assert_eq!(c.iter().filter(|x| x.family == "mod2").count(), 36);
        for case in c.iter().filter(|x| !x.port.is_empty()) {
            let (a, b) = (chain(case, false), chain(case, true));
            assert!(a.firmware_fx.ports_on().is_empty() && !a.firmware_fx.amp);
            assert!(
                b.firmware_fx.amp || !b.firmware_fx.ports_on().is_empty(),
                "{}",
                case.label
            );
        }
    }

    #[test]
    fn stimuli_are_deterministic_and_finite() {
        for k in ["impulse", "sweep", "noise", "notes"] {
            let (a, end) = stimulus(k, 4096);
            assert_eq!(a, stimulus(k, 4096).0);
            assert!(a.iter().all(|v| v.is_finite()) && end >= 1);
            assert!(a.iter().any(|v| *v != 0.0), "{k}");
        }
    }

    #[test]
    fn an_empty_chain_is_the_identity_and_the_measures_read_it() {
        let case = &cases()[0];
        let mut fx = chain(case, false);
        let (x, _) = stimulus("noise", 8192);
        let mut off = LayerFxChain::new(FS);
        off.firmware_fx = FirmwareFx::OFF;
        let y = render(&mut off, &x);
        assert!((rms_db(&y) - rms_db(&x)).abs() < 0.5);
        let _ = render(&mut fx, &x);
        assert!(centroid(&x) > 0.0);
        assert_eq!(bands_db(&x, &y).len(), BANDS.len());
    }
}
