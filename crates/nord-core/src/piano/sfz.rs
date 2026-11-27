//! SFZ text parser for `NpnoSound::from_sfz_str`: regions, groups, opcodes (moved out of `piano.rs`).

#[derive(Debug, Clone)]
pub(super) struct SfzRegionSpec {
    pub(super) default_path: String,
    pub(super) sample: Option<String>,
    pub(super) key_low: u8,
    pub(super) key_high: u8,
    pub(super) vel_low: u8,
    pub(super) vel_high: u8,
    pub(super) root_key: u8,
    pub(super) tune_cents: i8,
    pub(super) trigger_release: bool,
    pub(super) loop_start: Option<usize>,
    pub(super) loop_end: Option<usize>,
}

impl Default for SfzRegionSpec {
    fn default() -> Self {
        Self {
            default_path: String::new(),
            sample: None,
            key_low: 0,
            key_high: 127,
            vel_low: 1,
            vel_high: 127,
            root_key: 60,
            tune_cents: 0,
            trigger_release: false,
            loop_start: None,
            loop_end: None,
        }
    }
}

fn parse_sfz_note(raw: &str) -> Option<u8> {
    let trimmed = raw.trim();
    if let Ok(num) = trimmed.parse::<u8>() {
        return Some(num.min(127));
    }
    let lower = trimmed.to_ascii_lowercase();
    let mut chars = lower.chars().peekable();
    let base = match chars.next()? {
        'c' => 0i32,
        'd' => 2,
        'e' => 4,
        'f' => 5,
        'g' => 7,
        'a' => 9,
        'b' => 11,
        _ => return None,
    };
    let acc = match chars.peek().copied() {
        Some('#') => {
            chars.next();
            1
        }
        Some('b') => {
            chars.next();
            -1
        }
        _ => 0,
    };
    let oct_str: String = chars.collect();
    let oct: i32 = oct_str.parse().ok()?;
    let midi = (oct + 1) * 12 + base + acc;
    Some(midi.clamp(0, 127) as u8)
}

fn apply_sfz_opcode(spec: &mut SfzRegionSpec, key: &str, val: &str) {
    match key {
        "default_path" => spec.default_path = val.to_string(),
        "sample" => spec.sample = Some(val.to_string()),
        "key" => {
            if let Some(n) = parse_sfz_note(val) {
                spec.key_low = n;
                spec.key_high = n;
                spec.root_key = n;
            }
        }
        "lokey" => {
            if let Some(n) = parse_sfz_note(val) {
                spec.key_low = n;
            }
        }
        "hikey" => {
            if let Some(n) = parse_sfz_note(val) {
                spec.key_high = n;
            }
        }
        "pitch_keycenter" => {
            if let Some(n) = parse_sfz_note(val) {
                spec.root_key = n;
            }
        }
        "lovel" => {
            if let Ok(v) = val.parse::<u8>() {
                spec.vel_low = v.clamp(1, 127);
            }
        }
        "hivel" => {
            if let Ok(v) = val.parse::<u8>() {
                spec.vel_high = v.clamp(1, 127);
            }
        }
        "tune" => {
            if let Ok(c) = val.parse::<i16>() {
                spec.tune_cents = c.clamp(-128, 127) as i8;
            }
        }
        "trigger" => {
            spec.trigger_release = val.eq_ignore_ascii_case("release");
        }
        "loop_start" => {
            if let Ok(v) = val.parse::<usize>() {
                spec.loop_start = Some(v);
            }
        }
        "loop_end" => {
            if let Ok(v) = val.parse::<usize>() {
                spec.loop_end = Some(v);
            }
        }
        _ => {}
    }
}

pub(super) fn parse_sfz_regions(text: &str) -> Vec<SfzRegionSpec> {
    let mut regions = Vec::new();
    let mut global_spec = SfzRegionSpec::default();
    let mut group_spec = SfzRegionSpec::default();
    let mut current: Option<SfzRegionSpec> = None;
    let mut in_group = false;
    let mut in_global = false;

    for raw_line in text.lines() {
        let line = raw_line.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        // Scan tags and opcodes on the line
        let mut rest = line;
        while !rest.is_empty() {
            rest = rest.trim_start();
            if rest.starts_with('<') {
                if let Some(end_tag) = rest.find('>') {
                    let tag = &rest[..=end_tag];
                    if tag.eq_ignore_ascii_case("<global>") || tag.eq_ignore_ascii_case("<control>")
                    {
                        if let Some(r) = current.take() {
                            regions.push(r);
                        }
                        in_global = true;
                        in_group = false;
                    } else if tag.eq_ignore_ascii_case("<group>") {
                        if let Some(r) = current.take() {
                            regions.push(r);
                        }
                        group_spec = global_spec.clone();
                        in_group = true;
                        in_global = false;
                    } else if tag.eq_ignore_ascii_case("<region>") {
                        if let Some(r) = current.take() {
                            regions.push(r);
                        }
                        current = Some(if in_group {
                            group_spec.clone()
                        } else {
                            global_spec.clone()
                        });
                        in_global = false;
                    }
                    rest = &rest[end_tag + 1..];
                    continue;
                }
            }

            if let Some(eq_pos) = rest.find('=') {
                let key = rest[..eq_pos].trim().to_ascii_lowercase();
                let after_eq = &rest[eq_pos + 1..];
                let next_boundary = find_next_sfz_token(after_eq);
                let val = after_eq[..next_boundary].trim();
                if let Some(ref mut reg) = current {
                    apply_sfz_opcode(reg, &key, val);
                } else if in_group {
                    apply_sfz_opcode(&mut group_spec, &key, val);
                } else if in_global {
                    apply_sfz_opcode(&mut global_spec, &key, val);
                }
                rest = &after_eq[next_boundary..];
            } else {
                break;
            }
        }
    }
    if let Some(r) = current {
        regions.push(r);
    }
    regions
}

fn find_next_sfz_token(after_eq: &str) -> usize {
    let bytes = after_eq.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            return i;
        }
        if bytes[i].is_ascii_whitespace() {
            let tail = after_eq[i..].trim_start();
            if tail.is_empty() || tail.starts_with('<') {
                return i;
            }
            if let Some(next_space_or_eq) = tail.find(['=', ' ', '\t']) {
                if tail.as_bytes()[next_space_or_eq] == b'=' {
                    return i;
                }
            }
        }
        i += 1;
    }
    after_eq.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_opcodes_are_inherited_and_region_overrides() {
        let regions = parse_sfz_regions(
            "<group> lovel=1 hivel=64 loop_start=10 loop_end=20\n\
             <region> sample=a.wav key=60\n\
             <region> sample=b.wav lokey=62 hikey=64 pitch_keycenter=63 hivel=90",
        );
        assert_eq!(regions.len(), 2);
        assert_eq!(
            (regions[0].key_low, regions[0].key_high, regions[0].root_key),
            (60, 60, 60)
        );
        assert_eq!((regions[0].vel_low, regions[0].vel_high), (1, 64));
        assert_eq!(
            (regions[0].loop_start, regions[0].loop_end),
            (Some(10), Some(20))
        );
        assert_eq!(
            (regions[1].key_low, regions[1].key_high, regions[1].root_key),
            (62, 64, 63)
        );
        assert_eq!(regions[1].vel_high, 90);
    }

    #[test]
    fn note_names_parse_to_midi() {
        assert_eq!(parse_sfz_note("c4"), Some(60));
        assert_eq!(parse_sfz_note("a#3"), Some(58));
        assert_eq!(parse_sfz_note("72"), Some(72));
        assert_eq!(parse_sfz_note("x"), None);
    }
}
