//! 3-Layer Nord Wave 2 Derived Synth Engine (`DSPS` — PolyBLEP VA + ZDF Ladder Filter + LFO + Arp).
//!
//! Spec: `docs/specs/synth-engine.md`

use std::f32::consts::{PI, TAU};
use std::sync::Arc;

pub use crate::synth_sample::{LoadedSynthSample, SampleTimbre, SynthSampleSound};
use crate::synth_tables::{
    lfo_pitch_semitones, UNISON_VOICES, UNISON_VOICE_COUNT, UNISON_WEAK_CHANNEL_GAIN,
};
pub use crate::synth_types::{
    ArpMode, FilterModel, LfoDestination, LfoShape, NotePriority, SynthOscMode, VaWaveform,
    VibratoMode,
};
pub use crate::synth_wavetable::NUM_WAVETABLES;
use crate::synth_wavetable::{sample_wavetable, wavetable_bank};

mod arp;
mod arp_gate;
mod arp_order;
mod arp_pool;
pub mod arp_rate;
mod dsp;
pub mod filter_gain;
mod fw_adr_chain;
mod fw_adr_law;
mod fw_coef_memo;
mod fw_cutoff_env;
mod fw_env_rates;
mod fw_env_settled;
mod fw_filter;
pub mod fw_filter_law;
mod fw_filter_table_d;
mod fw_filter_tables;
mod fw_fm;
mod fw_knob_laws;
mod fw_ladder;
mod fw_ladder_tables;
mod fw_misc;
mod fw_shape;
mod fw_shape34;
mod fw_sine3;
mod fw_sine_hp;
mod fw_sine_warps;
mod fw_stage3;
mod fw_stage3_tables;
mod fw_stages;
mod fw_strip_cutoff;
mod fw_sub;
mod fw_super1;
mod fw_super2;
mod fw_sync;
mod fw_sync1;
mod fw_sync3;
mod fw_sync6;
mod fw_sync_table;
mod fw_velocity;
mod fw_wave;
pub(crate) mod fw_wave_tables;
mod gain_dezip;
mod hw_saw;
mod key_velocity;
mod measured;
mod p_block;
mod p_stage_words;
mod p_table;
mod p_type;
mod p_wave_mode;
mod p_words;
mod pattern;
mod pattern_presets;
mod perf;
mod render;
mod render_va;
mod sample_osc;
mod sound_init;
mod strip_map;
mod strip_path;
pub use arp_order::{extra_notes, zigzag_index, MAX_RANGE_EXTRA_THIRDS};
use dsp::unison_pan;
pub use dsp::{poly_blep, ZdfFilter, FM_RATIOS};
pub use fw_adr_chain::{fw_adr_step, FwAdrAction, FwAdrRecord, FW_ADR_LIMIT};
pub use fw_cutoff_env::{
    fw_cutoff_hz_of_knob, fw_env_cutoff_hz, fw_hz_of_r, fw_lfo_cutoff_hz, fw_lfo_semitones,
    fw_r_of_hz, fw_stage_r_of_knob, FW_R_CLIP, FW_STAGE_RATE_HZ,
};
pub use fw_env_rates::{
    attack_coeff, attack_time_passes, decay_release_coeff, rate_index_from_word, RateTables,
    DEFAULT_TABLES, LEGACY_TABLES, RATE_INDEX_MAX, SETTLED_TABLES,
};
pub use fw_filter::{
    cutoff_to_r, FilterEngine, FwCoefs, FwFilter, FwFilterType, TablePos, FW_CLIP_LIMIT,
    FW_CUTOFF_MAX_FRACTION, FW_CUTOFF_MIN_HZ,
};
pub use fw_fm::{
    fm_stage1, fm_stage2, fm_table, fm_table_for, partial_words as fm_partial_words, FmInputs,
    FmState, FmTable, FmVoice, TABLE_WORDS as FM_TABLE_WORDS,
};
pub use fw_knob_laws::{
    cutoff_word_for_knob, rate_index_for_knob, rate_word_for_knob, unit_word_for_knob, EnvKnobs,
    StripKnobs, DECAY_DISPLAY_TAU,
};
pub use fw_shape::{fw_shape_wave, fw_shape_width};
pub use fw_stages::{noise_sample, noise_step, saw_diff, spread_u, FwStage, FwStageState};
pub use fw_strip_cutoff::{fw_cutoff_cycles_per_sample, fw_strip_cutoff_r, FW_CUTOFF_CLIP};
pub use fw_super2::tables as boot_tables;
pub use fw_velocity::{
    amp_gain as fw_amp_velocity_gain, filter_factor as fw_filter_velocity_factor,
};
pub use fw_wave::{install_wave_tables, prepare_wave_bank, wave_table, WaveBankState};
pub use fw_wave_tables::{tables_from_code, tables_from_file, WAVE_TABLES};
pub use gain_dezip::{
    gain_pole_for, CONTROL_TICK_HZ, LEGACY_CONTROL_TICK_HZ, SY_FRAME_RATE_HZ, SY_GAIN_POLE,
};
pub use hw_saw::{firmware_blep_step, firmware_saw, firmware_saw_fade, SawModel};
pub use key_velocity::{
    key_velocity_g, key_velocity_g_fit, key_velocity_gain, KEY_CURVE_SATURATION, KEY_VELOCITY_G,
};
pub use measured::{
    supersaw_outer_cents, supersaw_voice_ratio, ENV_DECAY_TAU_FRACTION_MEASURED,
    FILTER_ENV_OCTAVES_MEASURED, OSC_ENV_PITCH_OCTAVES_MEASURED, SUPERSAW_OUTER_CENTS,
    SUPERSAW_VOICES, VIBRATO_DEPTH_SCALE_MEASURED,
};
pub use p_block::{PBlock, PSpec};
pub use p_stage_words::StageWords;
pub use p_table::{PTable, PTableSpec, PWave, PWAVE_COUNT};
pub use p_type::{row_type_cutoff_scale, row_type_cutoff_scale_with, type_cutoff_scale};
pub use p_words::{type_count, type_of, words_for, words_of_type, PWords, GAIN_PER_P16};
pub use pattern::{pan_gains, CustomPattern, PatternPan, ACCENT_VELOCITY_BOOST, PATTERN_MAX_STEPS};
pub use perf::{
    ClockDivision, GatePattern, PerfState, PATTERN_PRESETS, PATTERN_PRESET_COUNT,
    PATTERN_PRESET_MAX,
};
pub use sample_osc::{SampleInterpolation, SAMPLE_INTERPOLATION};
pub use strip_map::{
    attack_index, attack_seconds, cutoff_hz_from_word, decay_index, decay_seconds,
    strip_word_for_cutoff, STRIP_PASS_FRAMES,
};
use strip_path::StripKey;
pub use strip_path::{StripEnvRecords, StripParams, StripVoice, SynthChain};
pub use voice::SynthVoice;
#[cfg(test)]
mod tests_arp_walk;
#[cfg(test)]
mod tests_envelope;
#[cfg(test)]
mod tests_fw_filter_golden;
#[cfg(test)]
mod tests_fw_velocity;
#[cfg(test)]
#[rustfmt::skip]
mod tests_fw_fm_golden;
#[cfg(test)]
mod tests_fw_adr_law;
#[cfg(test)]
mod tests_fw_ladder_golden;
#[cfg(test)]
mod tests_fw_ladder_strip;
#[cfg(test)]
mod tests_fw_shape_golden;
#[cfg(test)]
mod tests_fw_stage3_golden;
#[cfg(test)]
mod tests_fw_stage_golden;
#[cfg(test)]
mod tests_fw_stage_golden10;
#[cfg(test)]
mod tests_fw_stage_golden11;
#[cfg(test)]
mod tests_fw_stage_golden2;
#[cfg(test)]
mod tests_fw_stage_golden3;
#[cfg(test)]
mod tests_fw_stage_golden4;
#[cfg(test)]
mod tests_fw_stage_golden5;
#[cfg(test)]
mod tests_fw_stage_golden6;
#[cfg(test)]
mod tests_fw_stage_golden7;
#[cfg(test)]
mod tests_fw_stage_golden8;
#[cfg(test)]
mod tests_fw_stage_golden9;
#[cfg(test)]
mod tests_hw_saw_golden;
#[cfg(test)]
mod tests_osc_lfo;
#[cfg(test)]
mod tests_pattern_edit;
#[cfg(test)]
mod tests_pattern_tie;
#[cfg(test)]
mod tests_perf;
#[cfg(test)]
mod tests_sample_pitch;
#[cfg(test)]
mod tests_strip;
mod voice;
mod voice_alloc;
pub mod voice_edges;

#[cfg(test)]
pub(crate) mod test_util {
    use super::*;

    pub(crate) fn render_n(synth: &mut SynthLayer, n: usize) -> Vec<[f32; 2]> {
        (0..n).map(|_| synth.render_frame()).collect()
    }

    pub(crate) fn energy(frames: &[[f32; 2]]) -> f32 {
        frames.iter().map(|f| f[0] * f[0] + f[1] * f[1]).sum()
    }

    pub(crate) fn samples_until_freq(s: &mut SynthLayer, target: f32) -> usize {
        for n in 0..400_000 {
            s.render_frame();
            let v = s.voices.iter().find(|v| v.active).unwrap();
            if (v.freq_hz - target).abs() < target * 0.001 {
                return n;
            }
        }
        panic!("glide never reached target");
    }
}

/// Maximum polyphonic voices per Synth layer. Pre-allocated with zero heap allocations during playback.
pub const MAX_SYNTH_VOICES: usize = 16;

/// ASSUMED: start phases of the four unison voices; they only decorrelate the voices.
pub(crate) const UNISON_START_PHASES: [f32; 4] = [0.0, 0.31, 0.57, 0.83];

/// Octaves of cutoff sweep at full Filter Env Amt.
///
/// EMPIRICAL: Audible Genius "laser zap" clip (LPM filter, Freq 468 Hz, Env Amt 7.4, env
/// 0.5 ms / 2.07 s, Pure Saw G4, resonance high): the resonance peak starts about 5-6 octaves above
/// 468 Hz, i.e. 7-8 octaves per full scale. Accuracy about +/-1 octave (peak snaps to harmonics of
/// the 392 Hz note, onset time uncertain). Replaces an inherited ASSUMED 3 octaves.
const FILTER_ENV_OCTAVES: f32 = 8.0;

/// Ratio of the audible filter and oscillator envelope decay time constant to the displayed Decay.
///
/// EMPIRICAL: filter envelope, same clip as above; the resonance peak, in octaves above the base cutoff, falls by a factor
/// 0.77 per 0.1 s (tau 0.38 s) while the display reads 2.07 s, so tau = 0.18 x display. One decay
/// value only; the shape (exponential in octaves) fits the nine tracked points within 10 %. The
/// oscillator envelope agrees: Audible Genius pitch-blip clip, displayed Decay 100 ms, pitch falls
/// x0.88..0.93 per 2 ms (tau 16-27 ms, predicted 18 ms). The amp envelope is NOT covered (its clip
/// settings had no decay); it keeps tau = displayed Decay.
const ENV_DECAY_TAU_FRACTION: f32 = 0.18;

/// Seconds the Delay-mode vibrato takes to fade from zero to full depth after its delay.
///
/// EMPIRICAL: 0.40..0.45 s measured on a real Stage 4 (Audible Genius "Delayed vibrato" clip,
/// Delay 0.7 s, f0 tracking); see `docs/research/synth-0x2f-ns4decode-crosscheck.md`.
const VIBRATO_DELAY_RAMP_SEC: f32 = 0.45;

/// Roland/Nord 7-saw detune spread coefficients relative to fundamental.
const SUPERSAW_DETUNES: [f32; 7] = [
    -0.11002313,
    -0.06288439,
    -0.01952356,
    0.0,
    0.01991221,
    0.06216538,
    0.10745242,
];

/// Initial phase offsets for the 7 SuperSaw oscillators to prevent constructive crests.
/// The 16 voices of the measured Super Saw (`measured_laws`) continue the sequence; the first
/// seven are the legacy values so the default render does not change.
const SUPERSAW_PHASE_INITS: [f32; SUPERSAW_VOICES] = [
    0.0, 0.24, 0.46, 0.68, 0.82, 0.15, 0.57, 0.33, 0.91, 0.07, 0.62, 0.39, 0.76, 0.21, 0.52, 0.88,
];

/// Single 16-Voice Polyphonic Synth Layer (`Synth A`, `Synth B`, or `Synth C`) with LFO and Arpeggiator.
#[derive(Debug, Clone)]
pub struct SynthLayer {
    pub sample_rate: f32,
    pub osc_mode: SynthOscMode,
    pub waveform: VaWaveform,
    /// Saw shape of `Saw` / `SuperSaw` (default `Classic`; `Firmware` is the `0x12501F` model).
    pub saw_model: SawModel,
    /// Smooth the voice gain like the SY DSP (default off, see `gain_dezip.rs`).
    pub gain_dezipper: bool,
    /// Gain targets latch at the measured 24 kHz ARM timer tick (default `true`, D131); `false`
    /// is the legacy 1 kHz tick (`--legacy-1khz-tick`). Only read when the de-zipper or the strip
    /// chain is on.
    pub measured_tick: bool,
    /// The decoded oscillator is in the ANALOG SHAPE category (`osc_category == 3`); only the
    /// strip chain reads it (`render.rs`).
    pub osc_shape_category: bool,
    /// Per-voice signal chain (default `Legacy`; `FirmwareStrip` is `strip_path.rs`).
    pub chain: SynthChain,
    /// Strip chain only: voice level from the key-down velocity series `g(vel) / 127` instead of the
    /// linear velocity (`key_velocity.rs`, default on; no effect on the legacy chain).
    pub strip_key_velocity: bool,
    /// Velocity from the ORG core 2 pass (`fw_velocity.rs`, `--firmware-synth-velocity`; default on since D142, off in `legacy`):
    /// the amp gain `V(setting, g)` replaces the linear `amp_velocity` blend and the filter-envelope
    /// depth follows `G / 128`, with `g` the ARM key byte of the velocity.
    pub firmware_velocity: bool,
    /// Strip chain only: the ANALOG SHAPE category plays as the firmware stage `0x1212D3` with this
    /// shape word (`fw_shape.rs`; the idle frame holds 0.5, the knob scale is open); `None` keeps
    /// the ASSUMED saw placeholder.
    pub strip_shape: Option<f32>,
    /// Strip chain only (opt-in): empirical oscillator-stage parameter block (`p_block.rs`).
    pub p_block: Option<PBlock>,
    /// Strip chain only (opt-in): the per-wave P table (`p_table.rs`); overrides `p_block` and also
    /// scales the oscillators that have no stage block of their own.
    pub p_table: Option<PTable>,
    /// Row of the table the decoded oscillator plays through (set from `(category, wave)`).
    pub p_wave: PWave,
    /// P words of the unison stage of the oscillator (MULTI, SUPER), used when the table is the
    /// verified-words one (`PTable::words`, `--synth-p verified`).
    pub stage_words: Option<StageWords>,
    /// ANALOG category / wave of the decoded oscillator as a modelled firmware stage (`fw_stages.rs`).
    pub strip_stage: Option<FwStage>,
    /// Strip chain only, opt-in (default off): play `strip_stage` (noise, Shape Square, SUPER, MULTI)
    /// with the firmware stage models instead of the decoded legacy waveform.
    pub strip_stages: bool,
    /// Strip chain only, opt-in (default `None`): the derived filter laws of [`fw_filter_law`]. `Some(u)`
    /// replaces the fitted row and type cutoff factors by the one factor `u` on the display Hz and
    /// maps the resonance through the firmware table position (`--synth-filter-law`).
    pub fw_filter_law: Option<f32>,
    /// Strip chain only, opt-in (default `None` = [`p_type::SUPER_LP24_TYPE_SCALE`] 4.4): the SUPER
    /// wave 0 / LP24 type factor (`--synth-super-lp24`, grid of D113).
    pub super_lp24_scale: Option<f32>,
    /// Strip chain only, default on since D223 (`--legacy-synth-resonance` turns it off): only the
    /// firmware resonance table position of [`fw_filter_law::resonance_position`]; implied by
    /// `fw_filter_law`. VERIFIED against the oracle stage for all 128 knobs (D214).
    pub fw_resonance: bool,
    /// Decoded: the raw `kFltType` of the layer is 3 ("LP+HP"), the stage `0x1257B2`.
    pub filter_raw3: bool,
    /// Opt-in (default `1.0`, `--synth-unison-gain`): factor on the per-voice gain of the analog
    /// unison sum (`1 / sqrt(N)` by default). The hardware lane weights are a unit-norm rotation
    /// (`docs/research/synth-unison-level.md`), the native sum keeps half that power per channel.
    pub unison_gain: f32,
    /// Opt-in (default off, `--synth-unison-weights`): the analog unison voices keep the fixed
    /// per-voice weights of the firmware lane mix (`cos` / `sin` of pi/8, no `1 / sqrt(N)`), i.e.
    /// `unison_gain` is applied to a per-voice gain of `1.0` instead of `1 / sqrt(N)`.
    pub unison_fw_weights: bool,
    /// Opt-in (default off, `--synth-filter-gain`): the filter output is scaled by the oracle gain
    /// of its type ([`filter_gain::filter_gain`], D195).
    pub filter_type_gain: bool,
    /// Opt-in (default off, `--synth-fw-env-law`): the filter envelope and the LFO reach the cutoff
    /// in the semitone domain (`fw_cutoff_env.rs`, oracle composition, D200 / D207): `2^(amount *
    /// 127 / 12 * level)` instead of `2^(amount * 8 * level)`, and `2^(128 lfo_amount lfo / 12)`
    /// instead of `1 + 0.8 lfo_amount lfo` on the Hz cutoff. Strip chain only.
    pub fw_env_law: bool,
    /// Opt-in (default off, `--synth-fw-adr-law`, D210): the oracle envelope law of `fw_adr_law.rs`:
    /// the amp attack index of a negative word is scaled by `g / 127` (key byte `g`) per voice and
    /// the decay records of both chains stop at the sustain level 0.5. Strip chain only.
    pub fw_adr_law: bool,
    /// Strip chain only, opt-in (default off, `--synth-stage3`): a `filter_raw3` layer runs the
    /// modelled stage `0x1257B2` (`fw_stage3.rs`) instead of the `Lp12` stand-in.
    pub fw_stage3: bool,
    /// Strip chain only, opt-in (default off): `Wavetable` layers play the firmware WAVE bank stage
    /// `0x1247B6` (`fw_wave.rs`, 320 of 320 oracle words) instead of the 64-table placeholder. Every
    /// wave plays the one boot table until the per-wave loader is decoded (OQ22).
    pub wave_bank: bool,
    /// Strip chain only, on by default since D57: the envelope rate indices come straight from the
    /// stored knobs (`strip_knobs`, `fw_knob_laws.rs`: index = knob - 64, VERIFIED run) instead of
    /// the ASSUMED displayed-time search of `strip_map.rs`. Layers without knobs keep the search.
    pub knob_laws: bool,
    /// Raw envelope knobs of the loaded program (set by the decoded-layer import).
    pub strip_knobs: Option<StripKnobs>,
    /// `SAMPLES` oscillator only (default on, D40): pick the 12-tap kernel with the firmware key
    /// rule (`sinc::key_rule::kernel_for_key`) instead of the ASSUMED ratio rule.
    /// VERIFIED: the voice update stores the played key (`[voice + 8]`) and sends the kernel base
    /// `0x9243B = 0xE4000 + 0xC00 k` for `k = key rule(key)` (`tools/arm_sy_voice.py`).
    pub sample_kernel_by_key: bool,
    /// `SAMPLES` oscillator only, opt-in (default off): the note-to-step ratio follows the firmware
    /// exponent tables (`sinc::pitch_law::key_ratio`) instead of exact `2^(n/12)`.
    pub sample_pitch_fw: bool,
    strip_cache: Option<(StripKey, StripParams)>,
    ctrl_counter: u32,
    pub pulse_width: f32,
    pub fm_amount: f32,
    pub fm_ratio: f32,
    pub fm_feedback: f32,
    pub fm_sub_ratio: f32,
    pub fm_sub_amount: f32,
    pub fm_env_amount: f32,
    pub supersaw_detune: f32,
    /// Opt-in: use the laws measured on the Audible Genius lesson clips (`measured.rs`): 16-voice
    /// Super Saw detune, filter envelope depth, decay time constant and Env To Pitch depth.
    /// Default `false` keeps the older scales (`render_hash` unchanged).
    pub measured_laws: bool,
    pub wavetable_index: u8,
    pub wavetable_position: f32,
    pub sample_timbre: SampleTimbre,
    pub filter_model: FilterModel,
    /// Filter implementation (default `Zdf`; `Firmware` is the Core2 stage family of `fw_filter.rs`).
    pub filter_engine: FilterEngine,
    pub cutoff_hz: f32,
    pub resonance: f32,
    pub filter_drive: f32,
    pub filter_env_amount: f32,
    /// Keyboard cutoff tracking (`0` = Off, `1` = 1/3, `2` = 2/3, `3` = 1/1).
    pub kb_track: u8,
    pub attack_sec: f32,
    pub decay_sec: f32,
    pub sustain_level: f32,
    pub release_sec: f32,

    // Dedicated filter envelope (ADR). When `filter_env_dedicated` is false the
    // filter envelope mirrors the amp decay (legacy behaviour).
    pub filter_env_dedicated: bool,
    pub filter_env_attack_sec: f32,
    pub filter_env_decay_sec: f32,
    pub filter_env_release_sec: f32,
    /// Velocity -> filter envelope depth (`0..1`; `0` = velocity ignored).
    /// ASSUMED: peak = `1 - amount * (1 - velocity)` (linear); plausible for a
    /// velocity-scaled depth; confirm by comparing filter sweep depth at vel 40 vs 127 on hardware.
    pub filter_env_vel_amount: f32,

    // Voice mode
    /// Unison setting `0..=3`. EMPIRICAL + firmware-VERIFIED voices: `1` = -2/+2 cents, `2` =
    /// -4/-1.32/+1.33/+4, `3` = -6/-2/+2/+6, no centre voice, each voice mostly in one channel;
    /// see `synth_tables::UNISON_VOICES`. Applies to Analog and Wavetable modes; FM and Sample
    /// modes ignore it (Sample unison uses a different detune set, unmodelled).
    pub unison: u8,
    pub mono: bool,
    pub legato: bool,
    /// Glide (portamento) in seconds per octave; `0` = off. Constant-rate in pitch: the
    /// manual says "the greater the distance between two notes, the longer the glide".
    /// ASSUMED: linear-in-semitones slew; the seconds-per-octave scale is not measured.
    /// Confirm by recording a one-octave glide at several Glide knob values.
    pub glide_sec: f32,
    pub note_priority: NotePriority,
    /// Fine + coarse tuning in cents.
    pub pitch_cents: f32,
    /// Amp velocity sensitivity `0..1` (`0` = velocity ignored).
    pub amp_velocity: f32,
    /// `false` bypasses the filter.
    pub filter_on: bool,
    // Oscillator envelope (ADR) modulating Osc Ctrl, or pitch with `osc_env_to_pitch`.
    pub osc_env_attack_sec: f32,
    pub osc_env_decay_sec: f32,
    pub osc_env_release_sec: f32,
    /// Bipolar amount `-1..1`.
    pub osc_env_amount: f32,
    pub osc_env_to_pitch: bool,
    pub osc_env_velocity: bool,
    /// Opt-in (`--synth-osc-env-morph`, D217): under `osc_env_to_pitch` the pitch excursion is the
    /// firmware's `48 * osc_env_dsp_word(amount * 60, true) * level` semitones
    /// (`synth_osc_env_word::osc_env_pitch_semitones`) instead of the `EMPIRICAL` linear depth.
    pub osc_env_fw_pitch: bool,
    // Vibrato
    pub vibrato_mode: VibratoMode,
    pub vibrato_rate_hz: f32,
    pub vibrato_depth_cents: f32,
    pub vibrato_delay_sec: f32,
    /// Controller value `0..1` for `Wheel`/`Aftertouch`/`Pedal` vibrato modes.
    pub vibrato_ctrl: f32,
    /// Live pitch-stick offset in cents, set by the engine pedal router (`0.0` = no bend).
    pub bend_cents: f32,
    /// System menu Fine Tune in cents (`0.0` = concert pitch, the default).
    pub master_tune_cents: f32,
    vib_phase: f32,
    /// Octave offset in octaves (`-3..=3`).
    /// ASSUMED: range clamp of +/-3 octaves; confirm against the panel Octave Shift range.
    pub octave_offset: i8,
    last_freq_hz: f32,

    // LFO Modulation
    pub lfo_shape: LfoShape,
    pub lfo_dest: LfoDestination,
    pub lfo_rate_hz: f32,
    pub lfo_amount: f32,
    lfo_phase: f32,
    lfo_sh_val: f32,

    // Arpeggiator & Gate
    pub arp_mode: ArpMode,
    pub arp_octave_range: u8,
    /// UP/DOWN walks the whole octave pool and back like the ARM pattern player (see `arp.rs`,
    /// `docs/research/arp-fw-vs-native.md`). On by default since D166; `SynthLayer::legacy` and
    /// `--legacy-arp` turn it off.
    pub arp_firmware_walk: bool,
    /// Stored RANGE value `0..=127` of the program (`apply_arp_modes`); the firmware walk reads its
    /// span while it still projects onto `arp_octave_range` and `perf.range_extra_thirds`.
    pub arp_range_raw: u8,
    /// Arp / Poly notes end after the firmware gate (2/3 of a step for the plain arpeggio and an
    /// `E` step, see `arp_gate.rs`) instead of lasting to the next step. On by default (D168): the
    /// ARM never writes the pattern object flag after its constructor, so 2/3 is the firmware
    /// state (`docs/research/arp-gate-flag-trace.md`); no hardware clip (OQ32).
    pub arp_firmware_gate: bool,
    pub arp_rate_bpm: f32,
    arp_step: usize,
    arp_sample_counter: f32,
    held_keys: [bool; 128],
    held_velocities: [u8; 128],
    held_gains: [f32; 128],
    arp_active_notes: [Option<u8>; MAX_SYNTH_VOICES],
    arp_rng: u32,
    /// Gate, Pattern, KB Hold, KB Sync and Master Clock sync (`perf.rs`).
    pub perf: PerfState,

    // Pre-decoded multisample instrument (`SAMPLES` mode)
    sample_sound: Option<Arc<SynthSampleSound>>,
    /// Shared sinc kernels for `SAMPLES` playback, built at construction (TD-08).
    sinc_kernels: Arc<crate::sinc::SincKernelSet>,

    // 16 Polyphonic Voices
    voices: [SynthVoice; MAX_SYNTH_VOICES],
}

impl SynthLayer {
    /// A layer on the legacy engine chain: no strip chain, no stage oscillators, no P table.
    /// `new` is the firmware default (D22); this is the A/B reference and what the legacy-path
    /// tests build on.
    pub fn legacy(sample_rate: f32) -> Self {
        Self {
            chain: SynthChain::Legacy,
            p_table: None,
            strip_stages: false,
            fw_filter_law: None,
            super_lp24_scale: None,
            fw_resonance: false,
            filter_raw3: false,
            unison_gain: 1.0,
            unison_fw_weights: false,
            filter_type_gain: false,
            fw_env_law: false,
            fw_adr_law: false,
            fw_stage3: false,
            firmware_velocity: false,
            arp_firmware_walk: false,
            arp_firmware_gate: false,
            ..Self::new(sample_rate)
        }
    }

    pub fn new(sample_rate: f32) -> Self {
        // Warm up the static mipmapped wavetable bank at construction time so render_frame never initializes it.
        let _ = wavetable_bank();
        Self {
            sample_rate: sample_rate.max(8000.0),
            osc_mode: SynthOscMode::Analog,
            waveform: VaWaveform::Saw,
            saw_model: SawModel::Classic,
            gain_dezipper: false,
            measured_tick: true,
            chain: SynthChain::FirmwareStrip,
            strip_key_velocity: true,
            firmware_velocity: true,
            strip_shape: None,
            p_block: None,
            p_table: Some(PTable::FITTED),
            p_wave: PWave::Other,
            stage_words: None,
            strip_stage: None,
            strip_stages: true,
            fw_filter_law: None,
            super_lp24_scale: None,
            fw_resonance: true,
            filter_raw3: false,
            unison_gain: 1.0,
            unison_fw_weights: false,
            filter_type_gain: false,
            fw_env_law: false,
            fw_adr_law: false,
            fw_stage3: false,
            wave_bank: false,
            knob_laws: true,
            strip_knobs: None,
            sample_kernel_by_key: true,
            sample_pitch_fw: false,
            osc_shape_category: false,
            strip_cache: None,
            ctrl_counter: 0,
            pulse_width: 0.5,
            fm_amount: 0.35,
            fm_ratio: 2.0,
            fm_feedback: 0.0,
            fm_sub_ratio: 3.0,
            fm_sub_amount: 0.0,
            fm_env_amount: 0.35,
            supersaw_detune: 0.3,
            measured_laws: false,
            wavetable_index: 0,
            wavetable_position: 0.0,
            sample_timbre: SampleTimbre::Off,
            filter_model: FilterModel::Ladder24,
            filter_engine: FilterEngine::Zdf,
            cutoff_hz: 2400.0,
            resonance: 0.3,
            filter_drive: 0.2,
            filter_env_amount: 0.0,
            kb_track: 0,
            attack_sec: 0.005,
            decay_sec: 0.3,
            sustain_level: 0.7,
            release_sec: 0.25,

            filter_env_dedicated: false,
            filter_env_attack_sec: 0.005,
            filter_env_decay_sec: 0.3,
            filter_env_release_sec: 0.25,
            filter_env_vel_amount: 0.0,

            unison: 0,
            mono: false,
            legato: false,
            glide_sec: 0.0,
            note_priority: NotePriority::Off,
            pitch_cents: 0.0,
            amp_velocity: 1.0,
            filter_on: true,
            osc_env_attack_sec: 0.005,
            osc_env_decay_sec: 0.3,
            osc_env_release_sec: 0.25,
            osc_env_amount: 0.0,
            osc_env_to_pitch: false,
            osc_env_velocity: false,
            osc_env_fw_pitch: false,
            vibrato_mode: VibratoMode::Off,
            vibrato_rate_hz: 5.0,
            vibrato_depth_cents: 0.0,
            vibrato_delay_sec: 0.0,
            vibrato_ctrl: 0.0,
            bend_cents: 0.0,
            master_tune_cents: 0.0,
            vib_phase: 0.0,
            octave_offset: 0,
            last_freq_hz: 0.0,

            lfo_shape: LfoShape::Triangle,
            lfo_dest: LfoDestination::FilterCutoff,
            lfo_rate_hz: 3.5,
            lfo_amount: 0.0,
            lfo_phase: 0.0,
            lfo_sh_val: 0.0,

            arp_mode: ArpMode::Off,
            arp_octave_range: 1,
            arp_firmware_walk: true,
            arp_range_raw: 0,
            arp_firmware_gate: true,
            arp_rate_bpm: 120.0,
            arp_step: 0,
            arp_sample_counter: 0.0,
            held_keys: [false; 128],
            held_velocities: [0; 128],
            held_gains: [1.0; 128],
            arp_active_notes: [None; MAX_SYNTH_VOICES],
            arp_rng: 0xA341_316C,
            perf: PerfState::default(),

            sample_sound: None,
            sinc_kernels: sample_osc::shared_sinc_kernels(),

            voices: [SynthVoice::default(); MAX_SYNTH_VOICES],
        }
    }

    /// Loads or clears a pre-decoded multisample instrument for `SynthOscMode::Sample`.
    /// Call outside the audio thread; playback reads the shared immutable buffer without locking or allocating.
    pub fn load_sample(&mut self, sound: Option<Arc<SynthSampleSound>>) {
        drop(self.replace_sample(sound));
    }

    /// [`Self::load_sample`] that returns the previous bank (see `PianoLayer::replace_sound`).
    pub fn replace_sample(
        &mut self,
        sound: Option<Arc<SynthSampleSound>>,
    ) -> Option<Arc<SynthSampleSound>> {
        for v in &mut self.voices {
            v.active = false;
        }
        std::mem::replace(&mut self.sample_sound, sound)
    }

    /// Returns the name of the currently loaded multisample sound, if any.
    pub fn loaded_sample_name(&self) -> Option<&str> {
        self.sample_sound.as_ref().map(|s| s.name.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_layer_has_all_voices_idle() {
        let s = SynthLayer::new(48_000.0);
        assert_eq!(s.voices.len(), MAX_SYNTH_VOICES);
        assert!(s.voices.iter().all(|v| !v.active));
        assert_eq!(FM_RATIOS.len(), 16);
    }
}
