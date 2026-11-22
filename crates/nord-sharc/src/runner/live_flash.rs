//! SMP flash feed of `sharc-live` (OQ41, D191): the commands `L PATH` and `F REC11`.
//!
//! `L PATH` loads a `.npno` (local file, never committed) as the flash image behind the external
//! banks (`core::periph::smpflash`) and switches the external-port chain model on. `F REC11` is
//! sent when the ARM has written a stroke descriptor: it places the file at the flash word the
//! descriptor law fixes (`SmpFlash::base_for`) and writes the linear page directory
//! `DM(0xB237B + i) = 65408 * i` for the groups the file spans, which stands in for the ARM's flash
//! file system (`docs/research/smp-page-refill.md`).
use crate::core::periph::smpflash::{SmpFlash, DIR_BASE, GROUP_WORDS};
use crate::core::Core;

/// Largest image accepted (the biggest factory `.npno` is 218 MB). ASSUMED cap, only a guard.
pub const MAX_BYTES: u64 = 512 << 20;

/// `L PATH`: loads the image and enables the chain model; returns the reply line.
pub fn load(core: &mut Core, path: &str) -> Result<String, String> {
    let len = std::fs::metadata(path)
        .map_err(|e| format!("L {path}: {e}"))?
        .len();
    if len > MAX_BYTES {
        return Err(format!("L {path}: {len} bytes exceeds {MAX_BYTES}"));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("L {path}: {e}"))?;
    if core.epchain().is_none() {
        core.enable_epchain();
    }
    let words = bytes.len() / 2;
    if let Some(e) = core.epchain() {
        e.flash = Some(SmpFlash::new(bytes));
    }
    Ok(format!("L words={words}"))
}

/// `F REC11`: places the loaded file for directory index `rec11` and writes the directory.
pub fn place(core: &mut Core, rec11: u32) -> Result<String, String> {
    let words = {
        let f = core
            .epchain()
            .and_then(|e| e.flash.as_mut())
            .ok_or("F: no flash image loaded (send L first)")?;
        f.set_base(SmpFlash::base_for(rec11));
        f.words()
    };
    let groups = (words + 74).div_ceil(u64::from(GROUP_WORDS)) as u32 + 2;
    for i in rec11..rec11.saturating_add(groups) {
        core.dm_write(DIR_BASE + i, SmpFlash::dir_value(i))
            .map_err(|e| format!("F: directory write {e:#x}"))?;
    }
    Ok(format!(
        "F base={} dir={}..{}",
        SmpFlash::base_for(rec11),
        rec11,
        rec11 + groups
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core() -> Core {
        Core::new(&vec![0u8; 0x24_0000])
    }

    fn temp_file(name: &str, bytes: &[u8]) -> String {
        let p = std::env::temp_dir().join(format!("live_flash_{}_{name}", std::process::id()));
        std::fs::write(&p, bytes).unwrap();
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn load_enables_the_flash_and_f_places_it_and_writes_the_directory() {
        let path = temp_file("a.bin", &[0u8; 4096]);
        let mut c = core();
        assert!(place(&mut c, 5).is_err(), "no image yet");
        assert_eq!(load(&mut c, &path).unwrap(), "L words=2048");
        let r = place(&mut c, 3).unwrap();
        assert!(r.starts_with("F base=196298 dir=3.."), "{r}");
        let _ = std::fs::remove_file(&path);
        assert_eq!(c.dm_read(DIR_BASE + 3), Ok(3 * GROUP_WORDS));
        assert_eq!(c.dm_read(DIR_BASE + 4), Ok(4 * GROUP_WORDS));
        assert_eq!(c.epchain().unwrap().flash.as_ref().unwrap().base(), 196_298);
    }

    #[test]
    fn a_missing_file_is_an_error() {
        assert!(load(&mut core(), "no/such/file.npno").is_err());
    }
}
