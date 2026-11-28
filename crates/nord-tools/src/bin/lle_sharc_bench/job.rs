//! One core of `lle_sharc_bench`: the `--core` argument, and the booted, armed job it makes.
use nord_sharc::core::UopBuildReport;
use nord_sharc::runner::live_presets::{boot_core, preset_named};
use nord_sharc::runner::msg::parse_msg_text;
use nord_sharc::runner::pass_sched::{Engine, PassJob};
use nord_sharc::runner::periph_run::PeriphOpts;
use nord_sharc::runner::uop_pass::{arm_for, ranges_for, RangeMode};

#[derive(Debug)]
pub struct CoreSpec {
    pub name: String,
    pub image: String,
    pub preset: String,
    pub msgs: Option<String>,
    pub tag: Option<String>,
}

/// `NAME=IMAGE,PRESET[,MSGFILE[,TAG]]`.
pub fn parse_core(s: &str) -> Result<CoreSpec, String> {
    let bad = || format!("bad --core {s:?} (want NAME=IMAGE,PRESET[,MSGFILE[,TAG]])");
    let (name, rest) = s.split_once('=').ok_or_else(bad)?;
    let f: Vec<&str> = rest.split(',').collect();
    if f.len() < 2 || f.len() > 4 {
        return Err(bad());
    }
    let opt = |i: usize| f.get(i).filter(|v| !v.is_empty()).map(|v| v.to_string());
    Ok(CoreSpec {
        name: name.into(),
        image: f[0].into(),
        preset: f[1].into(),
        msgs: opt(2),
        tag: opt(3),
    })
}

/// A job and, for the uop engines, what arming it built.
pub struct Armed {
    pub job: PassJob,
    pub build: Option<UopBuildReport>,
}

/// How a uop engine is armed: the `native::FAM_*` bit set, how the code is found and, for the probe,
/// the number of passes it runs.
#[derive(Clone, Copy, Debug)]
pub struct Arming {
    pub families: u32,
    pub ranges: RangeMode,
    pub probe_passes: u32,
}

pub fn job(spec: &CoreSpec, engine: Engine, arming: Arming) -> Result<Armed, String> {
    let data = std::fs::read(&spec.image).map_err(|e| format!("{}: {e}", spec.image))?;
    let preset = preset_named(&spec.preset).ok_or_else(|| format!("preset {:?}", spec.preset))?;
    let msgs = match &spec.msgs {
        Some(f) => parse_msg_text(&std::fs::read_to_string(f).map_err(|e| format!("{f}: {e}"))?)?,
        None => Vec::new(),
    };
    let boot = || boot_core(&data, &preset, &PeriphOpts::default());
    let mut core = boot()?;
    let mut engine = engine;
    let mut build = None;
    match (&spec.tag, engine) {
        (Some(tag), Engine::Aot) => {
            core.enable_aot_tag(tag)?;
        }
        (_, Engine::Uop | Engine::UopChecked | Engine::UopNative) => {
            let fail = |e: String| format!("{}: {e}", spec.name);
            let ranges = ranges_for(arming.ranges, boot, &preset, &msgs, arming.probe_passes)
                .map_err(fail)?;
            let idle_pc = preset.idle_pc;
            build = arm_for(
                &mut core,
                engine,
                arming.families,
                idle_pc,
                ranges.as_deref(),
            )
            .map_err(fail)?;
        }
        _ => engine = Engine::Interp,
    }
    let job = PassJob::from_preset(&spec.name, core, &preset, msgs, engine);
    Ok(Armed { job, build })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_specs_parse_with_optional_msgs_and_tag() {
        let c = parse_core("A=img.bin,org,m.txt,TAG").unwrap();
        assert_eq!(
            (c.name.as_str(), c.image.as_str(), c.preset.as_str()),
            ("A", "img.bin", "org")
        );
        assert_eq!(
            (c.msgs.as_deref(), c.tag.as_deref()),
            (Some("m.txt"), Some("TAG"))
        );
        let c = parse_core("B=img.bin,pno").unwrap();
        assert!(c.msgs.is_none() && c.tag.is_none());
        let c = parse_core("C=img.bin,org,,TAG").unwrap();
        assert!(c.msgs.is_none() && c.tag.as_deref() == Some("TAG"));
        assert!(parse_core("nope").is_err() && parse_core("D=only").is_err());
    }

    /// A synthetic image whose ORG entry (`0x125C88`) is `JUMP 0x125C88`, so the 100,000-step boot
    /// of the preset spins in place: no firmware, enough to test the arming of a job.
    fn spin_image(tag: &str) -> String {
        let mut image = vec![0u8; 0x12_5C88 * 2 + 6];
        for (k, p) in [0x063e_u16, 0x0012, 0x5C88].iter().enumerate() {
            let o = (0x12_5C88 + k) * 2;
            image[o..o + 2].copy_from_slice(&p.to_le_bytes());
        }
        let path = std::env::temp_dir().join(format!("uop_bench_{tag}_{}.bin", std::process::id()));
        std::fs::write(&path, image).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn spec(image: &str) -> CoreSpec {
        parse_core(&format!("T={image},org")).unwrap()
    }

    /// Arming with the extent scan: what the engine arms with unless told otherwise.
    const EXTENT: Arming = Arming {
        families: 0,
        ranges: RangeMode::Extent,
        probe_passes: 0,
    };

    #[test]
    fn a_uop_engine_arms_the_core_and_the_others_do_not() {
        let image = spin_image("arm");
        for engine in [Engine::Uop, Engine::UopChecked] {
            let a = job(&spec(&image), engine, EXTENT).unwrap();
            assert_eq!(a.job.engine, engine);
            assert!(a.build.is_some() && a.job.core.uop_enabled(), "{engine:?}");
            assert_eq!(
                a.job.cfg.idle_pc, 0x12_5DDF,
                "the poll loop of the org preset"
            );
        }
        let a = job(&spec(&image), Engine::Interp, EXTENT).unwrap();
        assert!(a.build.is_none() && !a.job.core.uop_enabled());
        // aot without a tag stays on the interpreter, as before
        let a = job(&spec(&image), Engine::Aot, EXTENT).unwrap();
        assert_eq!((a.job.engine, a.build.is_none()), (Engine::Interp, true));
        std::fs::remove_file(&image).unwrap();
    }

    #[test]
    fn a_probe_arms_the_core_from_the_pcs_of_its_passes_and_only_for_the_uop_engines() {
        let image = spin_image("probe");
        let probe = Arming {
            ranges: RangeMode::Probe,
            probe_passes: 1,
            ..EXTENT
        };
        let a = job(&spec(&image), Engine::Uop, probe).unwrap();
        assert!(a.build.is_some() && a.job.core.uop_enabled());
        // the probe ran on a second boot: the armed core did not take a step of it
        let plain = job(&spec(&image), Engine::Uop, EXTENT).unwrap();
        assert_eq!(a.job.core.steps, plain.job.core.steps);
        // the interpreter never probes: a probe request costs it nothing
        let a = job(&spec(&image), Engine::Interp, probe).unwrap();
        assert!(a.build.is_none() && !a.job.core.uop_enabled());
        std::fs::remove_file(&image).unwrap();
    }

    #[test]
    fn a_native_request_is_armed_or_refused_with_the_feature_hint() {
        let image = spin_image("native");
        match job(&spec(&image), Engine::UopNative, EXTENT) {
            Ok(a) => assert!(a.job.core.uop_enabled()),
            Err(e) => assert!(e.contains("uop-native") && e.contains("T:"), "{e}"),
        }
        std::fs::remove_file(&image).unwrap();
    }

    #[test]
    fn a_missing_image_is_an_error() {
        assert!(job(&spec("no/such/image.bin"), Engine::Uop, EXTENT).is_err());
    }
}
