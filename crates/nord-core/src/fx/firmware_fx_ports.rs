//! Per-port access to [`FirmwareFx`]: names, CLI list parsing and merging, shared by
//! `--firmware-fx-ports`, the GUI port checkboxes and the options summary.

use super::FirmwareFx;

/// Names of the optional ports in summary / CLI order. `amp` is not listed: it has its own
/// `--amp-path` switch.
pub const PORT_NAMES: [&str; 8] = [
    "delay-laws",
    "delay-time",
    "reverb-mix",
    "reverb",
    "compressor",
    "mod1",
    "mod2",
    "eq",
];

impl FirmwareFx {
    /// Mutable access to a port by its [`PORT_NAMES`] entry.
    pub fn port_mut(&mut self, name: &str) -> Option<&mut bool> {
        Some(match name {
            "delay-laws" => &mut self.delay_laws,
            "delay-time" => &mut self.delay_time,
            "reverb-mix" => &mut self.reverb_mix,
            "reverb" => &mut self.reverb,
            "compressor" => &mut self.compressor,
            "mod1" => &mut self.mod1,
            "mod2" => &mut self.mod2,
            "eq" => &mut self.eq,
            _ => return None,
        })
    }

    /// State of a port by name (`false` for an unknown name).
    pub fn port(&self, name: &str) -> bool {
        let mut c = *self;
        c.port_mut(name).is_some_and(|p| *p)
    }

    /// Names of the optional ports that are on, in [`PORT_NAMES`] order.
    pub fn ports_on(&self) -> Vec<&'static str> {
        PORT_NAMES.into_iter().filter(|n| self.port(n)).collect()
    }

    /// Switches on every port that is on in `other` (including `amp`); never switches one off.
    pub fn merge_on(&mut self, other: &FirmwareFx) {
        self.amp |= other.amp;
        self.pump |= other.pump;
        self.compressor_row |= other.compressor_row;
        self.mod1_unit_calls |= other.mod1_unit_calls;
        for n in PORT_NAMES {
            if other.port(n) {
                if let Some(p) = self.port_mut(n) {
                    *p = true;
                }
            }
        }
    }

    /// Parses a `--firmware-fx-ports` list (`reverb,mod1,eq`; `all` = every port, `parity` = every
    /// port but the reverb ones, [`FirmwareFx::PARITY`]) and switches
    /// those ports on.
    pub fn enable_ports(&mut self, list: &str) -> Result<(), String> {
        for item in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if item == "all" {
                self.merge_on(&FirmwareFx::ALL);
            } else if item == "parity" {
                self.merge_on(&FirmwareFx::PARITY);
            } else if let Some(p) = self.port_mut(item) {
                *p = true;
            } else {
                return Err(format!(
                    "invalid --firmware-fx-ports entry '{item}' (expected all|parity|{})",
                    PORT_NAMES.join("|")
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_port_has_a_distinct_field_and_all_is_complete() {
        let mut f = FirmwareFx::OFF;
        for n in PORT_NAMES {
            assert!(!f.port(n), "{n} starts off");
            *f.port_mut(n).unwrap() = true;
            assert!(f.port(n));
        }
        let all = FirmwareFx::ALL;
        assert!(f.ports_on().len() == PORT_NAMES.len() && PORT_NAMES.iter().all(|n| all.port(n)));
        assert!(f.port_mut("bogus").is_none() && !f.port("bogus"));
    }

    #[test]
    fn list_parsing_merges_and_rejects_unknown() {
        let mut f = FirmwareFx::LEGACY;
        f.enable_ports("reverb, mod2").unwrap();
        assert_eq!(f.ports_on(), ["reverb", "mod2"]);
        assert!(f.amp, "amp untouched");
        f.enable_ports("all").unwrap();
        assert_eq!(f, FirmwareFx::ALL);
        let mut p = FirmwareFx::OFF;
        p.enable_ports("parity").unwrap();
        assert_eq!(p.ports_on().len(), 6);
        assert!(p.amp && !p.reverb && !p.reverb_mix && p.mod2 && p.eq);
        let mut off = FirmwareFx::OFF;
        assert!(off.enable_ports("nope").is_err());
        let mut g = FirmwareFx::OFF;
        g.merge_on(&FirmwareFx::DEFAULT);
        assert!(
            g.amp && g.pump && g.ports_on().len() == 6 && !g.reverb,
            "the default is PARITY (D173, PUMP D134)"
        );
        let mut pre = FirmwareFx::OFF;
        pre.merge_on(&FirmwareFx::PRE_PARITY);
        assert!(
            pre.amp && pre.pump && pre.ports_on() == ["mod2"],
            "amp, PUMP and Mod 2 (D163, D134)"
        );
        let mut l = FirmwareFx::OFF;
        l.merge_on(&FirmwareFx::LEGACY);
        assert!(l.amp && l.pump && l.ports_on().is_empty());
    }
}
