//! The engines a runner can be asked for by name and the arming each one needs: the
//! [`UopConfig`] of an engine ([`uop_config`]), its command-line name ([`engine_named`]) and
//! [`arm_for`], which arms a booted core for it.
//!
//! OWNER: W1-RUN. Split from `arm.rs` (P11, files of at most 400 lines); the code ranges and the
//! stop pc are there.
use super::super::pass_sched::Engine;
use super::arm::{arm_uop, arm_uop_ranges};
use crate::core::{Core, UopBuildReport, UopConfig, UopError};

/// The `UopConfig` an engine runs with, `None` for the engines that do not use the uop path.
/// `families` is the `native::FAM_*` bit set of the inline families (0 = call-threaded only) and
/// only matters for [`Engine::UopNative`].
pub fn uop_config(engine: Engine, families: u32) -> Option<UopConfig> {
    match engine {
        Engine::Interp | Engine::Aot => None,
        Engine::Uop | Engine::UopChecked => Some(UopConfig::default()),
        Engine::UopNative => Some(UopConfig {
            native: true,
            families,
            ..UopConfig::default()
        }),
    }
}

/// [`arm_uop`] for the engine a runner was asked for: `Ok(None)` when it needs no arming. With
/// `ranges` the engine translates those short-word ranges instead of the [`super::code_ranges`] of
/// the core. The error is a message ready to print; a native request on a build without the feature
/// says how to enable it instead of silently measuring Tier 1.
pub fn arm_for(
    core: &mut Core,
    engine: Engine,
    families: u32,
    idle_pc: u32,
    ranges: Option<&[(u32, u32)]>,
) -> Result<Option<UopBuildReport>, String> {
    let Some(cfg) = uop_config(engine, families) else {
        return Ok(None);
    };
    let armed = match ranges {
        Some(r) => arm_uop_ranges(core, cfg, idle_pc, r),
        None => arm_uop(core, cfg, idle_pc),
    };
    match armed {
        Ok(report) => Ok(Some(report)),
        Err(UopError::NativeUnavailable) => {
            Err("the uop-native engine needs a build with feature \
             nord-sharc/uop-native (cargo run ... --features nord-sharc/uop-native)"
                .into())
        }
        Err(e) => Err(e.to_string()),
    }
}

/// The engine a command-line name selects: `interp`, `aot`, `uop`, `uop-checked`, `uop-native`.
pub fn engine_named(name: &str) -> Option<Engine> {
    Some(match name {
        "interp" => Engine::Interp,
        "aot" => Engine::Aot,
        "uop" => Engine::Uop,
        "uop-checked" => Engine::UopChecked,
        "uop-native" => Engine::UopNative,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::support::CODE;
    use super::*;
    use crate::core::support::core_with;

    #[test]
    fn engines_map_to_their_configuration_and_names() {
        assert_eq!(uop_config(Engine::Interp, 0), None);
        assert_eq!(uop_config(Engine::Aot, 0), None);
        assert_eq!(uop_config(Engine::Uop, 7), Some(UopConfig::default()));
        assert_eq!(
            uop_config(Engine::UopChecked, 0),
            Some(UopConfig::default())
        );
        let native = uop_config(Engine::UopNative, 5).unwrap();
        assert_eq!((native.native, native.families), (true, 5));
        for (name, engine) in [
            ("interp", Engine::Interp),
            ("aot", Engine::Aot),
            ("uop", Engine::Uop),
            ("uop-checked", Engine::UopChecked),
            ("uop-native", Engine::UopNative),
        ] {
            assert_eq!(engine_named(name), Some(engine));
        }
        assert_eq!(engine_named("Uop"), None);
        assert_eq!(engine_named(""), None);
    }

    #[test]
    fn arm_for_arms_the_uop_engines_only_and_explains_a_missing_native_tier() {
        let mut c = core_with(&[0x0001; 10]);
        assert_eq!(arm_for(&mut c, Engine::Interp, 0, CODE + 5, None), Ok(None));
        assert_eq!(arm_for(&mut c, Engine::Aot, 0, CODE + 5, None), Ok(None));
        assert!(!c.uop_enabled());
        let uop = arm_for(&mut c, Engine::Uop, 0, CODE + 5, None);
        assert!(uop.unwrap().is_some());
        assert!(c.uop_enabled());
        let mut n = core_with(&[0x0001; 10]);
        let native = arm_for(&mut n, Engine::UopNative, 0, CODE + 5, None);
        if cfg!(feature = "uop-native") {
            assert!(native.is_ok());
        } else {
            let e = native.unwrap_err();
            assert!(e.contains("uop-native") && e.contains("--features"), "{e}");
            assert!(!n.uop_enabled(), "a failed arming leaves the engine off");
        }
    }

    #[test]
    fn a_range_list_arms_the_engine_instead_of_the_extent_scan() {
        // ten NOPs, but only the first four are asked for
        let mut c = core_with(&[0x0001; 10]);
        let four = [(CODE, CODE + 4)];
        let narrow = arm_for(&mut c, Engine::Uop, 0, CODE + 4, Some(&four)).unwrap();
        let mut all = core_with(&[0x0001; 10]);
        let wide = arm_for(&mut all, Engine::Uop, 0, CODE + 4, None).unwrap();
        let (narrow, wide) = (narrow.unwrap(), wide.unwrap());
        assert!(c.uop_enabled() && all.uop_enabled());
        // the translator is real: four of the ten NOPs are asked for
        assert!(wide.insns > 0, "{wide:?}");
        assert!(narrow.insns < wide.insns, "{narrow:?} against {wide:?}");
        // a range outside the window is the engine's error, as text, and leaves the engine off
        let mut bad = core_with(&[0x0001; 10]);
        let e = arm_for(&mut bad, Engine::Uop, 0, CODE, Some(&[(0, 4)])).unwrap_err();
        assert!(e.contains("outside the code window"), "{e}");
        assert!(!bad.uop_enabled());
    }
}
