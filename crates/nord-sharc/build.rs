//! Build script: embeds the optional generated AOT block modules.
//!
//! `NORD_SHARC_AOT_HOT=<file.rs>` (an absolute path to the output of `sharc-aot --hot` or
//! `--ranges`) is copied to `$OUT_DIR/aot_hot_gen.rs` and compiled into `core::aot_hot`. Without
//! it a stub with no blocks is written, so ordinary builds (CI, `--all-features`) never need
//! firmware.
//!
//! `NORD_SHARC_AOT_HOT=TAG=<file.rs>;TAG2=<file2.rs>` embeds several modules, one per image and
//! tier (`sharc-aot --whole-pass`), each in its own namespace; `Core::enable_aot_tag("TAG")`
//! picks one (a pass benchmark runs one image per core, and their pcs overlap). The first module
//! is also the one `sharc_jit_diff --aot-top` enables.
use std::{env, fs, path::PathBuf};

const STUB: &str = "use crate::core::aot_hot::AotBlock;\n\
pub(super) fn blocks() -> &'static [AotBlock] {\n    &[]\n}\n\
pub(super) fn by_tag(_: &str) -> Option<&'static [AotBlock]> {\n    None\n}\n\
pub(super) fn tags() -> &'static [&'static str] {\n    &[]\n}\n";

/// Source of a combined module for `TAG=file;TAG=file`.
fn multi(spec: &str) -> String {
    let mut src = String::from("use crate::core::aot_hot::AotBlock;\n");
    // blocks() is the first module: what sharc_jit_diff --aot-top enables
    src.push_str("pub(super) fn blocks() -> &'static [AotBlock] {\n    m0::blocks()\n}\n");
    let (mut arms, mut tags) = (String::new(), Vec::new());
    for (i, part) in spec.split(';').filter(|p| !p.is_empty()).enumerate() {
        let (tag, path) = part
            .split_once('=')
            .unwrap_or_else(|| panic!("NORD_SHARC_AOT_HOT part {part:?} is not TAG=file"));
        println!("cargo:rerun-if-changed={path}");
        let path = path.replace('\\', "/");
        src.push_str(&format!("mod m{i} {{\n    include!(r#\"{path}\"#);\n}}\n"));
        arms.push_str(&format!("        {tag:?} => Some(m{i}::blocks()),\n"));
        tags.push(format!("{tag:?}"));
    }
    src.push_str(&format!(
        "pub(super) fn by_tag(tag: &str) -> Option<&'static [AotBlock]> {{\n    match tag {{\n{arms}        _ => None,\n    }}\n}}\n"
    ));
    src.push_str(&format!(
        "pub(super) fn tags() -> &'static [&'static str] {{\n    &[{}]\n}}\n",
        tags.join(", ")
    ));
    src
}

fn main() {
    println!("cargo:rerun-if-env-changed=NORD_SHARC_AOT_HOT");
    println!("cargo:rerun-if-changed=build.rs");
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("aot_hot_gen.rs");
    match env::var("NORD_SHARC_AOT_HOT") {
        Ok(spec) if spec.contains('=') => fs::write(&out, multi(&spec)).expect("write multi"),
        Ok(path) if !path.is_empty() => {
            println!("cargo:rerun-if-changed={path}");
            let mut src = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("NORD_SHARC_AOT_HOT={path}: {e}"));
            src.push_str(
                "pub(super) fn by_tag(_: &str) -> Option<&'static [AotBlock]> {\n    None\n}\n\
pub(super) fn tags() -> &'static [&'static str] {\n    &[]\n}\n",
            );
            fs::write(&out, src).expect("write single");
        }
        _ => fs::write(&out, STUB).expect("write stub"),
    }
}
