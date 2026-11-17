//! Start-up presets of the chips `sharc-live` can serve (D138, D144): the constants of `sharc-sys`
//! (`--pno`, default ORG) as data, so one binary serves PNO core 1, and the organ program that runs
//! on ORG core 1, ORG core 2 and PNO core 2 (loader entry `0x125C88`, `dsp-image-hunt.md` s5.17).
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
    }
}

/// Preset by its `sharc-live` flag name (`pno`, `org`).
pub fn preset_named(name: &str) -> Option<Preset> {
    match name {
        "pno" => Some(pno_preset()),
        "org" => Some(org_preset()),
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
        assert!(preset_named("sy").is_none());
    }
}
