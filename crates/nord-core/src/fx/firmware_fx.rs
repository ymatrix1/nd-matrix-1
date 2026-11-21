//! The opt-in switch that routes the engine's FX parameters through the firmware closed forms.

/// Which firmware-derived behaviours a [`super::LayerFxChain`] uses. `Default` is [`Self::DEFAULT`]:
/// only `amp` is on (the one switch verified end to end, D22); [`Self::OFF`] restores the engine
/// paths everywhere.
///
/// Status of each switch (`docs/specs/effects-and-routing.md` s4.0c):
///
/// * `delay_laws`: the delay's send and dry gains follow `kDlyMix` (`fw_map::delay_send`,
///   `delay_dry`; VERIFIED words, EMPIRICAL roles) instead of the engine's `dry + wet * mix`.
///   The feedback law (`v / 127`) equals the engine scale, so only the `0.95` stability clamp
///   remains.
/// * `delay_time`: the free-running delay time follows the firmware law of `kDlyRateClkDiv`
///   (`fw_delay_time`: 14 bit value to `c[0x15]` samples, first echo `c[0x15] + 2` samples
///   later). Both halves VERIFIED (ARM harness, `sharc-fx` oracle). It needs
///   `LayerFxChain::delay_rate_raw` (the preset's 14 bit value, set by `apply_fx_chain`); without
///   it, or in tempo sync, the engine time law is used. The firmware cross-fades two taps for
///   45 ms when the value changes; the engine keeps its own slew.
/// * `reverb_mix`: the reverb send and dry gains follow `kRvbAmt` (`fw_map::reverb_send` / `reverb_dry`,
///   equal power; words VERIFIED on all 128 values, role EMPIRICAL). Only the mix: the engine's
///   reverb network keeps its own taps, decay and damping (the multi-tap routines are not ported).
/// * `reverb`: the Spring reverbs (`kRvbType` 0 and 1, `ReverbModel::Spring` and its variation)
///   run the firmware routine `0x12417E` with the ported core `0x123F61` ([`super::FwReverb`],
///   bit exact against the `sharc-fx` oracle, `tests_fw_rvb_golden.rs`) instead of the engine FDN.
///   The routine does the dry/wet mix itself (`kRvbAmt` words), so `reverb_mix` is not applied on
///   top. **Latency 4 frames** while it runs; the other reverb types keep the engine FDN (their
///   routines are not ported). Default off: see `docs/research/fx-reverb-leslie.md` s10 for the
///   comparison with the video.
/// * `compressor`: the layer compressor is the firmware gain computer ([`super::FwCompressor`])
///   with the `kCmpAmt` words. Algorithm VERIFIED, `kCmpAmt` words VERIFIED, slope and log gain
///   ASSUMED. **Latency 4 frames (83 us at 48 kHz)** on that layer; the stage runs even when
///   `comp_enabled` is false (amount 0, unity) so every layer of the mix keeps the same latency.
///
/// * `amp`: the Twin, Twin II, JC, JC II, Small and Small II amps run the firmware drive stage
///   ([`super::FwAmpDrive`] with the real tables and the words of `kAmpTypeNew`, `kAmpDrive` =
///   `127 * drive / 10`) instead of the engine's `tanh` drive and RBJ cabinet voicing. Words,
///   tables and algorithm VERIFIED against the oracle (`tests_amp_golden.rs`); **latency 4 frames**
///   on that layer. The firmware's cabinet FIR and the organ unit's tone routines are not
///   modelled, so the engine's 3-band EQ still follows; To Rotary, LP24 and HP24 keep the engine
///   path (their type switch sends no drive stage words).
///
/// * `mod1`: the Mod 1 types A-PAN (`kFx1Type` 0, 1) and TREM (2, 3) run the firmware routines
///   `0x122D93` ([`super::FwAutoPan`]) and `0x122C39` ([`super::FwAllpass`] at mix 0) with the
///   instance words of the FX image and the `kFx1Amt` / `kFx1RateClkDiv` words
///   (`fw_mod1`; `docs/research/fx-mod-types.md`). Routines VERIFIED against the oracle, words
///   VERIFIED, the TREM role EMPIRICAL. The LFO runs at the firmware frame rate (Hz equal), the
///   rate scale is 0.03 .. 5.3 Hz. **Latency 4 frames** on that layer. Free running only
///   (Master Clock sync keeps the engine path); RM, A-WAH, WAH and PUMP keep the engine models.
///
/// * `eq`: the Amp EQ (`kAmpBass`, `kAmpMid`, `kAmpMidFreq`, `kAmpTreble`) runs the ported tone
///   stack `0x1243F4` ([`super::FwToneStack2`]) with the ARM's words (`fw_eq`,
///   `docs/research/fx-eq-words.md`) instead of the engine's three biquads. Routine and words
///   VERIFIED; the joint mid words, the interpolation between captured indices and the
///   organ unit's routine layout are ASSUMED; the recurrence runs at the engine rate (the
///   firmware's is 43,402 Hz, so frequencies are 1.106 times higher at 48 kHz). **Latency 2
///   frames** on that layer. The compressor (`compressor`) needs no extra switch for
///   `kCmpResponse`: the firmware ignores it, and so does this path.
///
/// Not switchable yet, because the audio path needs words or tables that are not decoded: the
/// tone stack (`FwToneStack2`, base words of the Amp type switch), the all-pass (`FwAllpass`, Fx1 type to engine `Mod1Type` map) and the
/// delay line itself (`FwDelay` / the ring variant `0x120AB5`: ring length and routing words).
/// Their parameter fills exist as `fw_params::set_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirmwareFx {
    /// Delay mix law of `kDlyMix`.
    pub delay_laws: bool,
    /// Firmware compressor with the `kCmpAmt` words.
    pub compressor: bool,
    /// Firmware delay time law of `kDlyRateClkDiv` (free-running delay only).
    pub delay_time: bool,
    /// Reverb mix law of `kRvbAmt`.
    pub reverb_mix: bool,
    /// Firmware reverb routine for the Spring types (`kRvbType` 0, 1); other types use the FDN.
    pub reverb: bool,
    /// Firmware Amp drive stage with the `kAmpTypeNew` words (Twin, JC, Small families).
    pub amp: bool,
    /// Firmware A-PAN (Fx1 types 0, 1) and TREM (types 2, 3) with the `kFx1Amt` /
    /// `kFx1RateClkDiv` words.
    pub mod1: bool,
    /// Firmware PUMP (Fx1 types 10, 11, `0x122D2C`), on by default (D134): follows the Master
    /// Clock when the program syncs to it (counter tick plus one cycle per division), free runs
    /// otherwise. `false` (`--legacy-pump`) restores the engine pump model (`fx/mod1_dynamic.rs`).
    /// Not in `FX_PORT_NAMES`: like `amp` it has its own opt-out. Independent of `mod1`.
    pub pump: bool,
    /// Firmware Amp EQ (tone stack `0x1243F4` with the `kAmpTreble` / `kAmpBass` / `kAmpMid` /
    /// `kAmpMidFreq` words) in place of the engine's three biquads.
    pub eq: bool,
    /// Firmware Mod 2 (`kFx2Type` 0 to 11: `PHAS`, `FLANG`, `CHOR`, `VIBE`, `ENS`, `SPIN`) with the
    /// `kFx2Amt` / `kFx2Rate` words. `ENS` runs with ASSUMED circular tap registers (D95).
    pub mod2: bool,
    /// Firmware Amp `LP24` / `HP24` (state variable pair `0x124EFA`, `kAmpMidFreq` / `kAmpMid`
    /// words) in place of the engine's two biquad pairs. Opt-in (`--firmware-fx-ports lp24`), not in
    /// [`Self::ALL`] or [`FX_PORT_NAMES`](super::FX_PORT_NAMES): no clip confirms it yet
    /// (`docs/research/amp-types-789.md`). Latency 4 frames on an LP24 / HP24 layer.
    pub lp24: bool,
    /// Default since D187: the compressor context carries the firmware's own gain-law words (SLOPE 0.75
    /// / 1.0, REDUCTION_SCALE 1/96, TABLE1 2^-16, LOG_GAIN 6.0, read from the FX image row and not
    /// written by the ARM or the routine) instead of the ASSUMED ones (`fw_params::CMP_SLOPE`).
    /// Only read with `compressor`. Not in `FX_PORT_NAMES`: it is a refinement of the `compressor`
    /// port, on in `ALL` and `PARITY`; `--legacy-compressor-row` and `--legacy-fx-ports` turn it off.
    pub compressor_row: bool,
    /// Opt-in (D186): the firmware RM / A-WAH / WAH / PUMP stage runs the frames per call of the
    /// layer's unit (`LayerFxChain::mod1_unit_frames`: 2 for the organ and piano units, 4 for the
    /// synth units, the ARM's block count words times two frames) instead of
    /// `LayerFxChain::mod1_rest_frames`. Only read with `mod1`. Not in `FX_PORT_NAMES`, not in `ALL`.
    pub mod1_unit_calls: bool,
}

impl Default for FirmwareFx {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl FirmwareFx {
    /// The engine default since D173: [`Self::PARITY`], every firmware FX port but the two reverb
    /// ports (`--legacy-fx-ports` restores [`Self::PRE_PARITY`], `--legacy-mod2` also turns Mod 2
    /// off). Mod 2 has been on since D163 and the amp earlier; D173 adds the delay laws and time,
    /// the compressor, Mod 1 and the EQ after the owner's approval of 2026-11-20
    /// (`docs/research/program-ranking-fx.md` s7).
    pub const DEFAULT: Self = Self::PARITY;

    /// The default of D163 to D170: the amp and Mod 2. What `--legacy-fx-ports` restores.
    pub const PRE_PARITY: Self = Self {
        amp: true,
        pump: true,
        mod2: true,
        ..Self::OFF
    };

    /// The default before D163 (`amp` and `pump`), what `--legacy-all` and `--legacy-mod2` restore.
    pub const LEGACY: Self = Self {
        amp: true,
        pump: true,
        ..Self::OFF
    };

    /// Every switch off.
    pub const OFF: Self = Self {
        delay_laws: false,
        compressor: false,
        delay_time: false,
        reverb_mix: false,
        reverb: false,
        amp: false,
        mod1: false,
        pump: false,
        eq: false,
        mod2: false,
        lp24: false,
        compressor_row: false,
        mod1_unit_calls: false,
    };

    /// Every firmware port except the two reverb ports: delay laws and time, compressor, Mod 1,
    /// EQ, Mod 2 and the amp. The default since D173 (`--firmware-fx-ports parity` names it).
    ///
    /// EMPIRICAL: the set `docs/research/program-ranking-fx.md` found to explain the ranked
    /// distance of the worst 20 programs (mean score 18.14 to 2.19 with this set switched on,
    /// against 0 for `ALL`); the reverb ports are left out because the owner hears the firmware
    /// reverb worse than the engine reverb on A:21 (`docs/research/firmware-flag-audit.md`).
    pub const PARITY: Self = Self {
        reverb_mix: false,
        reverb: false,
        ..Self::ALL
    };

    /// Every switch on.
    pub const ALL: Self = Self {
        delay_laws: true,
        compressor: true,
        delay_time: true,
        reverb_mix: true,
        reverb: true,
        amp: true,
        mod1: true,
        pump: true,
        eq: true,
        mod2: true,
        lp24: false,
        compressor_row: true,
        mod1_unit_calls: false,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_pump() -> bool {
        std::hint::black_box(FirmwareFx::ALL).pump
    }

    #[test]
    fn default_is_off_and_all_turns_every_switch_on() {
        assert_eq!(FirmwareFx::default(), FirmwareFx::DEFAULT);
        let d = std::hint::black_box(FirmwareFx::DEFAULT);
        assert!(
            d.amp && d.pump && d.delay_laws && d.compressor,
            "PARITY is the default (D173)"
        );
        assert!(
            !FirmwareFx::OFF.pump && all_pump(),
            "PUMP is on by default (D134)"
        );
        assert!(d.mod2, "the firmware Mod 2 is the default since D163");
        let pre = std::hint::black_box(FirmwareFx::PRE_PARITY);
        assert!(pre.amp && pre.mod2 && !pre.delay_laws && !pre.compressor && !pre.eq && !pre.mod1);
        let legacy = std::hint::black_box(FirmwareFx::LEGACY);
        assert!(!legacy.mod2 && legacy.amp && legacy.pump);
        let all = std::hint::black_box(FirmwareFx::ALL);
        assert!(
            all.delay_laws
                && all.compressor
                && all.delay_time
                && all.reverb_mix
                && all.reverb
                && all.amp
                && all.mod1
                && all.eq
                && all.mod2
        );
        assert!(!d.reverb, "the firmware reverb is opt-in");
        assert!(
            d.mod1 && all.mod1,
            "the firmware Mod 1 is on by default since D173"
        );
        assert!(all.mod2, "the firmware Mod 2 is part of ALL");
        assert!(
            d.eq && all.eq,
            "the firmware EQ is on by default since D173"
        );
        assert_ne!(all, FirmwareFx::OFF);
    }

    #[test]
    fn parity_is_all_without_the_reverb_ports_and_is_the_default() {
        let p = std::hint::black_box(FirmwareFx::PARITY);
        assert!(p.delay_laws && p.delay_time && p.compressor && p.mod1 && p.eq && p.mod2 && p.amp);
        assert!(!p.reverb && !p.reverb_mix);
        assert_eq!(p, FirmwareFx::DEFAULT);
        assert_ne!(p, FirmwareFx::ALL);
        assert_ne!(p, FirmwareFx::PRE_PARITY);
    }
}
