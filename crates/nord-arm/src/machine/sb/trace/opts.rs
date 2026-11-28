//! OWNER: W1-FLOW. `BuildOpts`: the trace-formation limits and pass switches of `trace::build`.

use super::super::config::SbConfig;

/// Trace-formation limits and pass switches (a copy of the matching `SbConfig` fields).
#[derive(Clone, Debug)]
pub struct BuildOpts {
    pub max_uops: usize,
    pub max_guards: usize,
    pub max_pages: usize,
    pub inline_depth: u8,
    pub fuse: bool,
    pub predicate: bool,
    pub dead_flags: bool,
    pub fold_literals: bool,
}

impl From<&SbConfig> for BuildOpts {
    fn from(c: &SbConfig) -> Self {
        Self {
            max_uops: c.max_uops,
            max_guards: c.max_guards,
            max_pages: c.max_pages,
            inline_depth: c.inline_depth,
            fuse: c.fuse,
            predicate: c.predicate,
            dead_flags: c.dead_flags,
            fold_literals: c.fold_literals,
        }
    }
}

impl Default for BuildOpts {
    fn default() -> Self {
        Self::from(&SbConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_options_copy_the_configuration() {
        let c = SbConfig {
            max_uops: 3,
            max_guards: 2,
            max_pages: 1,
            inline_depth: 0,
            fuse: true,
            predicate: true,
            dead_flags: true,
            fold_literals: true,
            ..SbConfig::default()
        };
        let o = BuildOpts::from(&c);
        assert_eq!(
            (o.max_uops, o.max_guards, o.max_pages, o.inline_depth),
            (3, 2, 1, 0)
        );
        assert!(o.fuse && o.predicate && o.dead_flags && o.fold_literals);
        let d = BuildOpts::default();
        assert!(!d.fuse && d.max_uops == 128);
    }
}
