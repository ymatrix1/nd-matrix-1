//! Start-up presets of the chips `sharc-live` can serve (D138, D144, D153): the constants of `sharc-sys`
//! (`--pno`, default ORG) as data, so one binary serves PNO core 1, and the organ program that runs
//! on ORG core 1, ORG core 2 and PNO core 2 (loader entry `0x125C88`, `dsp-image-hunt.md` s5.17).
use super::fx_run::{FxCore, FX1, FX2};
use super::pass::{fault_text, run_steps};
use super::periph_run::PeriphOpts;
use super::system::PNO_PING_PONG;
use crate::core::Core;

/// Start-up of one core program: entry, poll loop, mailbox, board gates.
#[derive(Clone, Debug)]
pub struct Preset {
    pub entry: u32,
    pub idle_pc: u32,
    pub trigger: u32,
    pub boot: u64,
    pub irq: Option<(u8, u32)>,
    pub ping_pong: Option<(u32, u32, u32)>,
    pub devs: Vec<(u32, u32)>,
    pub flag1: bool,
    /// The SY1SY2 sample chip: FLAG12 high and the interrupt-driven pass of `live_sy` (D153).
    pub sy: bool,
    /// The SMP piano chip (`live_smp`, OQ41): the SY model with its own FIFO and ring tap.
    pub smp: bool,
    /// An FX chip core: [`FxDrive`] runs before every pass (peer counter, ping selector).
    pub fx: Option<FxCore>,
}

/// VERIFIED: the `--pno` preset of `sharc-sys` (`pno-full-pass.md`): entry `0x121007`, poll loop
/// `0x121091`, mailbox `0xB00E1`, board gates.
pub fn pno_preset() -> Preset {
    Preset {
        entry: 0x12_1007,
        idle_pc: 0x12_1091,
        trigger: 0xB_00E1,
        boot: 3_000_000,
        irq: Some((15, 1)),
        ping_pong: Some(PNO_PING_PONG),
        devs: vec![
            (0x28ac_0380, 1),
            (0x3108_d008, 5),
            (0x3108_c06c, 0x400),
            (0x3108_c00c, 4),
        ],
        flag1: true,
        sy: false,
        smp: false,
        fx: None,
    }
}

/// The organ-program start-up, shared with `sharc-sys` (its defaults are built from this value, so
/// the two cannot drift). VERIFIED: ORG core 2 runs 16,575 idle passes under it (`org-system-run.md`,
/// `synth-e2e-oracle-vs-native.md`): entry `0x125C88`, poll loop `0x125DDF`, mailbox `0xB00E2`,
/// boot 100,000 instructions, gates `0x282C0380 = 1` and `0x3108D008 = 5`. EMPIRICAL: PNO core 2
/// has the same loader entry `0x125C88` and idles every pass under it (`pno-jit-arm-profile.md`
/// s5, `arm-dsp-live-link-synth.md`: 2,441 of 2,441 passes); that it is the same program is
/// inferred from the entry and the block sizes (`dsp-image-hunt.md` s5.17).
pub fn org_preset() -> Preset {
    Preset {
        entry: 0x12_5C88,
        idle_pc: 0x12_5DDF,
        trigger: 0xB_00E2,
        boot: 100_000,
        irq: None,
        ping_pong: None,
        devs: vec![(0x282c_0380, 1), (0x3108_d008, 5)],
        flag1: false,
        sy: false,
        smp: false,
        fx: None,
    }
}

/// The SY1SY2 start-up of the `sharc-ref` scenarios (`sy_isr_refill.scn`): `warmup 0x1c0b2d
/// 100000 flag12=1`. VERIFIED: the image reaches its idle loop and runs 400 audio interrupts
/// under it (`sy-smp-audio.md` s2); it has no poll loop, so `idle_pc` and `trigger` are unused.
pub fn sy_preset() -> Preset {
    Preset {
        entry: 0x1C_0B2D,
        idle_pc: 0,
        trigger: 0,
        boot: 100_000,
        irq: None,
        ping_pong: None,
        devs: vec![],
        flag1: false,
        sy: true,
        smp: false,
        fx: None,
    }
}

/// The SMP start-up of the `smp_voice_ref` scenarios (`warmup 0x1c11e0 100000 flag12=1`,
/// `poke 0x5201 0x40`, `fifo 0x92196`), VERIFIED against the oracle runs of `smp-voice.md` s5.
/// ASSUMED: the ARM-driven image needs nothing more than the scenarios gave it.
pub fn smp_preset() -> Preset {
    Preset {
        entry: super::live_smp::ENTRY,
        sy: true,
        smp: true,
        ..sy_preset()
    }
}

/// Preset of one FX core from its descriptor (entry, poll loop, mailbox, gates, FLAG1, receive
/// interrupt), the values `sharc-fxlive` boots with (`fx-oracle-live.md` s2, VERIFIED from the
/// listings). The boot length is the `sharc-fxlive` default; ASSUMED enough for both cores.
fn fx_preset(fx: FxCore) -> Preset {
    Preset {
        entry: fx.entry,
        idle_pc: fx.idle_pc,
        trigger: fx.trigger,
        boot: 3_000_000,
        irq: fx.irq,
        ping_pong: None,
        devs: fx.devs.to_vec(),
        flag1: fx.flag1,
        sy: false,
        smp: false,
        fx: Some(fx),
    }
}

/// FX Core1 start-up (`--irq P4I:0x41`, FLAG1, `FX1.devs`).
pub fn fx1_preset() -> Preset {
    fx_preset(FX1)
}

/// FX Core2 start-up (mailbox `0xC02C2`, Core1 hand-over counter played by the drive).
pub fn fx2_preset() -> Preset {
    fx_preset(FX2)
}

/// Preset by its `sharc-live` flag name (`pno`, `org`).
pub fn preset_named(name: &str) -> Option<Preset> {
    match name {
        "pno" => Some(pno_preset()),
        "org" => Some(org_preset()),
        "sy" => Some(sy_preset()),
        "smp" => Some(smp_preset()),
        "fx1" => Some(fx1_preset()),
        "fx2" => Some(fx2_preset()),
        _ => None,
    }
}

/// Boots `data` with `preset` (peripheral models attached first, as `sharc-sys` does).
pub fn boot_core(data: &[u8], preset: &Preset, periph: &PeriphOpts) -> Result<Core, String> {
    let mut core = Core::new(data);
    periph.attach(&mut core);
    core.pc = preset.entry;
    if preset.flag1 {
        core.set_flag_input(1, true);
    }
    if preset.sy {
        core.set_flag_input(12, true);
    }
    if preset.smp {
        // IOP register, plain memory in the model (`sharc-ref` `poke 0x5201 0x40`).
        let _ = core.dm_write(super::live_smp::STATUS_ADDR, super::live_smp::STATUS_VALUE);
    }
    for &(a, v) in &preset.devs {
        core.preset_device(a, v);
    }
    match run_steps(&mut core, preset.boot) {
        Some(f) => Err(format!("boot faulted: {}", fault_text(&f))),
        None => Ok(core),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pno_preset_matches_the_sharc_sys_constants() {
        let p = pno_preset();
        assert_eq!(
            (p.entry, p.idle_pc, p.trigger),
            (0x12_1007, 0x12_1091, 0xB_00E1)
        );
        assert_eq!(p.irq, Some((15, 1)));
        assert_eq!(p.devs.len(), 4);
    }

    #[test]
    fn org_preset_matches_the_sharc_sys_defaults() {
        let p = org_preset();
        assert_eq!(
            (p.entry, p.idle_pc, p.trigger, p.boot),
            (0x12_5C88, 0x12_5DDF, 0xB_00E2, 100_000)
        );
        assert!(p.irq.is_none() && p.ping_pong.is_none() && !p.flag1);
        assert_eq!(p.devs, vec![(0x282c_0380, 1), (0x3108_d008, 5)]);
    }

    #[test]
    fn presets_are_selected_by_name() {
        assert_eq!(preset_named("org").unwrap().entry, 0x12_5C88);
        assert_eq!(preset_named("pno").unwrap().entry, 0x12_1007);
        assert!(preset_named("sy").unwrap().sy && !preset_named("org").unwrap().sy);
        let smp = preset_named("smp").unwrap();
        assert!(smp.sy && smp.smp && smp.entry == 0x1C_11E0);
        assert!(!preset_named("sy").unwrap().smp);
        assert!(preset_named("nope").is_none());
        let (f1, f2) = (preset_named("fx1").unwrap(), preset_named("fx2").unwrap());
        assert_eq!(
            (f1.entry, f1.idle_pc, f1.trigger),
            (0x12_11E3, 0x12_128B, 0xC_02C1)
        );
        assert_eq!(
            (f2.idle_pc, f2.trigger, f2.irq),
            (0x12_125B, 0xC_02C2, None)
        );
        assert!(f1.flag1 && f1.irq == Some((15, 0x41)) && !f2.flag1);
        assert!(f2.fx.unwrap().peer && !f1.fx.unwrap().peer);
        assert!(preset_named("org").unwrap().fx.is_none());
    }
}
