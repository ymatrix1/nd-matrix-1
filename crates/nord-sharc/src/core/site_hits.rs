//! Opt-in execution counter for a list of program counters (the census sites of
//! `docs/research/sharc-census-diff.md`).
//!
//! Off by default: [`Core::step`] only tests whether the list is empty. Switched on by
//! [`Core::count_sites`] or by the environment variable `NS4_SITE_HITS` (comma separated hex
//! short-word addresses, read by [`Core::new`]); with the variable set, a core prints its
//! non-zero counts to stderr when it is dropped (`site_hits pc=0x.. count=N`), so every oracle
//! binary can report them without a new option. Only the interpreter counts: the JIT and the AOT
//! blocks do not go through [`Core::step`].
use super::Core;

/// Environment variable that lists the sites to count.
pub const ENV: &str = "NS4_SITE_HITS";

/// Parses `0x1211C4,1211C7` (hex, optional `0x`) into addresses; bad items are skipped.
pub fn parse_sites(text: &str) -> Vec<u32> {
    text.split(',')
        .filter_map(|t| {
            let t = t.trim();
            u32::from_str_radix(t.strip_prefix("0x").unwrap_or(t), 16).ok()
        })
        .collect()
}

impl Core {
    /// Starts counting the executions of each short-word address in `pcs` (replaces the list).
    pub fn count_sites(&mut self, pcs: &[u32]) {
        self.site_hits = pcs.iter().map(|&p| (p, 0)).collect();
    }

    /// `(pc, executions)` for every counted site, in the order given to [`Core::count_sites`].
    pub fn site_hits(&self) -> &[(u32, u64)] {
        &self.site_hits
    }

    /// Adds the environment list (if any) to a fresh core.
    pub(super) fn sites_from_env(&mut self) {
        if let Ok(text) = std::env::var(ENV) {
            let pcs = parse_sites(&text);
            self.count_sites(&pcs);
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn bump_site(&mut self) {
        let pc = self.pc;
        for s in &mut self.site_hits {
            if s.0 == pc {
                s.1 += 1;
            }
        }
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        if self.site_hits.iter().any(|s| s.1 != 0) && std::env::var_os(ENV).is_some() {
            for (pc, n) in self.site_hits.iter().filter(|s| s.1 != 0) {
                eprintln!("site_hits pc=0x{pc:x} count={n}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_hex_with_and_without_prefix_and_skips_junk() {
        assert_eq!(parse_sites("0x10, 1F,zz,"), vec![0x10, 0x1F]);
    }

    #[test]
    fn counts_executions_of_listed_pcs_only() {
        // NOP parcels (1) from short address 0
        let mut image = vec![0u8; 64];
        for k in 0..16 {
            image[2 * k] = 1;
        }
        let mut c = Core::new(&image);
        c.count_sites(&[0, 2, 0x100]);
        for _ in 0..4 {
            c.step().unwrap();
        }
        assert_eq!(c.site_hits(), &[(0, 1), (2, 1), (0x100, 0)]);
    }

    #[test]
    fn empty_list_counts_nothing() {
        let mut c = Core::new(&[1, 0, 1, 0]);
        c.step().unwrap();
        assert!(c.site_hits().is_empty());
    }
}
