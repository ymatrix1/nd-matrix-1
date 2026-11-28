//! Real-data check for issue #678: the Synth SUSTPED bit of every factory program reaches
//! `pedals.sustped` when the program is loaded with `synth_sustain_program` on.
//!
//! Needs the factory backup (`Nord Stage 4 Factory Restore RevK v1.66.ns4b`, vendor data, never
//! committed, D20 / D128) in `.reference/`, or the path in `NS4_FACTORY_BACKUP`. Without it the
//! test returns early, so CI stays green. Derived numbers: `docs/research/synth-sustped.md`.

use nord_core::cbin::{CbinFile, Ns4BackupArchive, ProgramDecoded};
use nord_core::engine::Stage4Engine;
use std::io::BufReader;
use std::path::PathBuf;

const BACKUP_NAME: &str = "Nord Stage 4 Factory Restore RevK v1.66.ns4b";

fn backup_path() -> Option<PathBuf> {
    let p = std::env::var_os("NS4_FACTORY_BACKUP")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../.reference")
                .join(BACKUP_NAME)
        });
    p.is_file().then_some(p)
}

/// VERIFIED: the five programs whose Synth bit is 0 (`docs/research/synth-sustped.md` s1,
/// `python tools/synth_sustped.py`), as (name stem, layer index).
const EXCEPTIONS: [(&str, usize); 5] = [
    ("Cathedral Pipes", 0),
    ("Wormhole Organ", 0),
    ("J3 Split Special", 0),
    ("Forever Split", 1),
    ("ScissorSplit Whl", 2),
];

#[test]
fn factory_program_bits_reach_pedals_sustped_on_load() {
    let Some(path) = backup_path() else {
        eprintln!("skipped: factory backup not found in .reference/ or NS4_FACTORY_BACKUP");
        return;
    };
    let archive = Ns4BackupArchive::open(&path).expect("open backup");
    let mut file = BufReader::new(std::fs::File::open(&path).unwrap());
    let mut engine = Stage4Engine::new(48_000.0);
    engine.synth_sustain_program = true;
    let (mut programs, mut set, mut off) = (0usize, [0usize; 3], Vec::new());
    for e in archive.entries_with_ext(".ns4p") {
        let bytes = archive.read_entry(&mut file, &e.name).expect("read entry");
        let Ok(cbin) = CbinFile::parse(&bytes) else {
            continue;
        };
        let Ok(prog) = ProgramDecoded::from_cbin_file(&cbin) else {
            continue;
        };
        engine.apply_program(&prog);
        let bits = prog.global.synth_sustain;
        assert_eq!(engine.pedals.sustped[4..7], bits, "{}", e.name);
        programs += 1;
        for (i, b) in bits.iter().enumerate() {
            set[i] += usize::from(*b);
            if !*b {
                off.push((e.name.clone(), i));
            }
        }
    }
    assert_eq!(programs, 384);
    assert_eq!(set, [381, 383, 383]);
    for (stem, layer) in EXCEPTIONS {
        assert!(
            off.iter().any(|(n, i)| n.contains(stem) && *i == layer),
            "{stem} layer {layer} should be off"
        );
    }
    assert_eq!(off.len(), 5);
}
