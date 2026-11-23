//! Several `.ns4b` banks open at once: merged catalog, duplicate detection, per-entry routing.
//!
//! The first opened bank is the *primary*: Organize, Export and Live import stay tied to it.
//! Every further bank is merged into the drawer / panel catalog. An entry of a later bank is
//!
//! * dropped when an entry with the same name **and** the same CRC-32 + size is already listed
//!   (an exact duplicate, e.g. the same factory sound in two backups);
//! * listed under its own name when the name is new;
//! * listed as `<folder>/<name> [<bank>].<ext>` when the name is taken by different bytes, so the
//!   two sounds never shadow each other (the folder is kept: the piano type comes from it).
//!
//! The catalog key of an entry is what the panel and drawer pass back to the loaders; `origin`
//! maps a key of a later bank to `(bank index, real entry name)`. Primary keys are real names.

use super::{piano_catalog, BackupLibraryState};
use nord_core::cbin::{Ns4BackupArchive, ProgramSoundRefs};
use nord_core::piano::PianoCategory;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// CRC-32 and uncompressed size of an archive entry (the duplicate fingerprint).
type Fingerprint = (u32, u64);

/// A bank merged after the primary one.
pub(super) struct ExtraBank {
    pub(super) path: PathBuf,
    pub(super) archive: Arc<Ns4BackupArchive>,
}

/// What `add_backup_path` did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct BankMerge {
    /// The file was already open (same path): nothing changed.
    pub already_open: bool,
    /// It became the primary bank (nothing was open before).
    pub primary: bool,
    /// Entries listed under their own name.
    pub added: usize,
    /// Entries listed under a `[bank]` name because different bytes own the plain name.
    pub renamed: usize,
    /// Entries skipped because an identical entry (same name, CRC-32 and size) is listed.
    pub duplicates: usize,
}

impl BankMerge {
    /// One-line status text.
    pub fn summary(&self, label: &str) -> String {
        if self.already_open {
            format!("{label} is already open")
        } else if self.primary {
            format!("Opened {label}")
        } else {
            format!(
                "Added {label}: {} new, {} renamed, {} duplicates skipped",
                self.added, self.renamed, self.duplicates
            )
        }
    }
}

/// How one incoming entry was merged.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Merged {
    /// Listed under its own name.
    Added(String),
    /// Listed under this other name (the plain name is taken by different bytes).
    Renamed(String),
    /// Identical to the entry already listed under this key; not listed again.
    Duplicate(String),
}

/// `dir/stem [label].ext` of `name`, with `-2`, `-3`... after the label until `taken` is false.
fn renamed(name: &str, label: &str, taken: impl Fn(&str) -> bool) -> String {
    let (dir, file) = name.rsplit_once('/').map_or(("", name), |(d, f)| (d, f));
    let (stem, ext) = file.rsplit_once('.').map_or((file, ""), |(s, e)| (s, e));
    let make = |n: u32| {
        let tag = if n == 1 {
            label.to_string()
        } else {
            format!("{label}-{n}")
        };
        let mut s = String::new();
        if !dir.is_empty() {
            s.push_str(dir);
            s.push('/');
        }
        s.push_str(&format!("{stem} [{tag}]"));
        if !ext.is_empty() {
            s.push('.');
            s.push_str(ext);
        }
        s
    };
    (1..)
        .map(make)
        .find(|candidate| !taken(candidate))
        .expect("an unbounded counter always finds a free name")
}

/// Merges `incoming` `(name, fingerprint)` entries of the bank `label` into `listed`
/// (`key -> fingerprint` of everything already in the catalog; new keys are inserted).
pub(super) fn merge_entries(
    listed: &mut HashMap<String, Fingerprint>,
    incoming: &[(String, Fingerprint)],
    label: &str,
) -> Vec<Merged> {
    let mut out = Vec::with_capacity(incoming.len());
    for (name, fp) in incoming {
        match listed.get(name) {
            None => {
                listed.insert(name.clone(), *fp);
                out.push(Merged::Added(name.clone()));
            }
            Some(have) if have == fp => out.push(Merged::Duplicate(name.clone())),
            Some(_) => {
                // A renamed copy may exist from an earlier merge of the same bytes: reuse it.
                let twin = renamed(name, label, |c| listed.get(c).is_some_and(|f| f != fp));
                if listed.get(&twin) == Some(fp) {
                    out.push(Merged::Duplicate(twin));
                } else {
                    listed.insert(twin.clone(), *fp);
                    out.push(Merged::Renamed(twin));
                }
            }
        }
    }
    out
}

fn fingerprint_of(archive: &Ns4BackupArchive, name: &str) -> Option<Fingerprint> {
    archive
        .entries()
        .iter()
        .find(|e| e.name == name)
        .map(|e| (e.crc32, e.uncompressed_size))
}

fn bank_label(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("bank")
        .to_string()
}

impl BackupLibraryState {
    /// Every open bank, primary first.
    pub fn bank_paths(&self) -> Vec<&Path> {
        self.backup_path
            .iter()
            .map(PathBuf::as_path)
            .chain(self.extra_banks.iter().map(|b| b.path.as_path()))
            .collect()
    }

    /// Closes every bank and empties the catalog.
    pub fn close_banks(&mut self) {
        for (_, arc) in self.piano_cache.drain() {
            self.retired_pianos.push(arc);
        }
        for (_, arc) in self.sample_cache.drain() {
            self.retired_samples.push(arc);
        }
        self.extra_banks.clear();
        self.origin.clear();
        self.key_by_real.clear();
        self.archive = None;
        self.backup_path = None;
        self.manifest_summary = None;
        self.programs.clear();
        self.pianos.clear();
        self.piano_types.clear();
        self.samples.clear();
        self.program_slots.clear();
        self.preset_slots.clear();
        self.layout_rev = self.layout_rev.wrapping_add(1);
        self.status_message = "No .ns4b backup loaded".to_string();
        self.error_message = None;
        self.reap_retired_sounds();
    }

    /// Opens `path` as the primary bank, or merges it into the open ones (see the module doc).
    pub fn add_backup_path(&mut self, path: &Path) -> Result<BankMerge, String> {
        let label = bank_label(path);
        if self.archive.is_none() {
            self.open_backup_path(path)?;
            return Ok(BankMerge {
                primary: true,
                ..Default::default()
            });
        }
        let same = |p: &Path| match (p.canonicalize(), path.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => p == path,
        };
        if self.bank_paths().into_iter().any(same) {
            self.status_message = BankMerge {
                already_open: true,
                ..Default::default()
            }
            .summary(&label);
            return Ok(BankMerge {
                already_open: true,
                ..Default::default()
            });
        }

        let archive = Ns4BackupArchive::open(path).map_err(|e| format!("{e}"))?;
        let mut reader = BufReader::new(
            File::open(path).map_err(|e| format!("failed to reopen '{}': {e}", path.display()))?,
        );
        let bank = self.extra_banks.len() + 1;
        let mut listed = self.listed_fingerprints();
        let mut report = BankMerge::default();

        let incoming = |ext: &str| -> Vec<(String, Fingerprint)> {
            archive
                .entries_with_ext(ext)
                .into_iter()
                .map(|e| (e.name.clone(), (e.crc32, e.uncompressed_size)))
                .collect()
        };
        let mut apply = |me: &mut Self, names: Vec<(String, Fingerprint)>| {
            let merged = merge_entries(&mut listed, &names, &label);
            let mut keys = Vec::new();
            for ((real, _), m) in names.iter().zip(merged) {
                let key = match m {
                    Merged::Added(k) => {
                        report.added += 1;
                        keys.push((real.clone(), k.clone()));
                        k
                    }
                    Merged::Renamed(k) => {
                        report.renamed += 1;
                        keys.push((real.clone(), k.clone()));
                        k
                    }
                    Merged::Duplicate(k) => {
                        report.duplicates += 1;
                        k
                    }
                };
                me.key_by_real.insert((bank, real.clone()), key);
            }
            for (real, key) in &keys {
                me.origin.insert(key.clone(), (bank, real.clone()));
            }
            keys
        };

        let program_keys = apply(self, incoming(".ns4p"));
        let sample_keys = apply(self, incoming(".nsmp4"));
        let piano_names = incoming(".npno");
        let piano_keys = apply(self, piano_names);

        // Piano types come from each entry's header, in hardware order.
        let models = piano_catalog::read_models(&archive, &mut reader);
        let type_of: HashMap<&str, PianoCategory> = models
            .iter()
            .map(|m| (m.entry.as_str(), m.category))
            .collect();
        for (real, key) in piano_keys {
            let cat = type_of
                .get(real.as_str())
                .copied()
                .or_else(|| PianoCategory::from_entry_path(&real))
                .unwrap_or(PianoCategory::Grand);
            self.pianos.push(key);
            self.piano_types.push(cat);
        }
        self.programs
            .extend(program_keys.into_iter().map(|(_, k)| k));
        self.programs.sort();
        self.samples.extend(sample_keys.into_iter().map(|(_, k)| k));
        self.samples.sort();

        self.extra_banks.push(ExtraBank {
            path: path.to_path_buf(),
            archive: Arc::new(archive),
        });
        self.error_message = None;
        self.status_message = report.summary(&label);
        Ok(report)
    }

    /// `key -> fingerprint` of every entry in the catalog.
    fn listed_fingerprints(&self) -> HashMap<String, Fingerprint> {
        self.programs
            .iter()
            .chain(&self.pianos)
            .chain(&self.samples)
            .filter_map(|key| {
                let (_, archive, real) = self.source_of(key)?;
                Some((key.clone(), fingerprint_of(&archive, &real)?))
            })
            .collect()
    }

    /// Index of the bank that owns catalog `key` (`0` = primary).
    pub(super) fn bank_of(&self, key: &str) -> usize {
        self.origin.get(key).map_or(0, |(b, _)| *b)
    }

    /// `(file, archive, real entry name)` behind catalog `key`.
    pub(super) fn source_of(&self, key: &str) -> Option<(PathBuf, Arc<Ns4BackupArchive>, String)> {
        match self.origin.get(key) {
            Some((b, real)) => {
                let bank = self.extra_banks.get(b - 1)?;
                Some((bank.path.clone(), Arc::clone(&bank.archive), real.clone()))
            }
            None => Some((
                self.backup_path.clone()?,
                Arc::clone(self.archive.as_ref()?),
                key.to_string(),
            )),
        }
    }

    /// Resolves sound references (`f` runs per archive) to catalog keys, asking the bank `owner`
    /// first and the other open banks for whatever it lacks, so a program finds its piano / sample
    /// even when the sound sits in another bank.
    pub(super) fn resolve_refs(
        &self,
        owner: usize,
        f: impl Fn(&Ns4BackupArchive) -> ProgramSoundRefs,
    ) -> ProgramSoundRefs {
        let mut banks: Vec<&Ns4BackupArchive> = self.archive.iter().map(|a| a.as_ref()).collect();
        banks.extend(self.extra_banks.iter().map(|b| b.archive.as_ref()));
        let order = std::iter::once(owner)
            .chain((0..banks.len()).filter(|i| *i != owner))
            .filter(|i| *i < banks.len());
        let mut out = ProgramSoundRefs::default();
        for b in order {
            let found = f(banks[b]);
            let key = |n: Option<String>| {
                n.map(|n| self.key_by_real.get(&(b, n.clone())).cloned().unwrap_or(n))
            };
            let fill = |slot: &mut Option<String>, v: Option<String>| {
                if slot.is_none() {
                    *slot = key(v);
                }
            };
            fill(&mut out.piano_a, found.piano_a);
            fill(&mut out.piano_b, found.piano_b);
            fill(&mut out.synth_a, found.synth_a);
            fill(&mut out.synth_b, found.synth_b);
            fill(&mut out.synth_c, found.synth_c);
        }
        out
    }
}

#[cfg(test)]
#[path = "banks_tests.rs"]
mod bank_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(n: u64) -> Fingerprint {
        (0, n)
    }

    fn inc(items: &[(&str, u64)]) -> Vec<(String, Fingerprint)> {
        items.iter().map(|(n, s)| (n.to_string(), fp(*s))).collect()
    }

    #[test]
    fn merge_skips_identical_keeps_new_and_renames_conflicts() {
        let mut listed = HashMap::from([("P/a.ns4p".to_string(), fp(1))]);
        let r = merge_entries(
            &mut listed,
            &inc(&[("P/a.ns4p", 1), ("P/b.ns4p", 2), ("P/a.ns4p", 3)]),
            "bank2",
        );
        assert_eq!(r[0], Merged::Duplicate("P/a.ns4p".into()));
        assert_eq!(r[1], Merged::Added("P/b.ns4p".into()));
        // Second `a` with other bytes cannot reuse the plain name.
        assert_eq!(r[2], Merged::Renamed("P/a [bank2].ns4p".into()));
        assert_eq!(listed.len(), 3);
    }

    #[test]
    fn same_conflicting_bank_merged_twice_does_not_grow() {
        let mut listed = HashMap::from([("P/a.ns4p".to_string(), fp(1))]);
        let first = merge_entries(&mut listed, &inc(&[("P/a.ns4p", 3)]), "b");
        let again = merge_entries(&mut listed, &inc(&[("P/a.ns4p", 3)]), "b");
        assert_eq!(first[0], Merged::Renamed("P/a [b].ns4p".into()));
        assert_eq!(again[0], Merged::Duplicate("P/a [b].ns4p".into()));
        assert_eq!(listed.len(), 2);
    }

    #[test]
    fn two_banks_with_same_label_get_counter_suffix() {
        let mut listed = HashMap::from([("P/a.ns4p".to_string(), fp(1))]);
        merge_entries(&mut listed, &inc(&[("P/a.ns4p", 2)]), "b");
        let r = merge_entries(&mut listed, &inc(&[("P/a.ns4p", 3)]), "b");
        assert_eq!(r[0], Merged::Renamed("P/a [b-2].ns4p".into()));
    }

    #[test]
    fn renamed_keeps_folder_and_handles_no_dir_or_ext() {
        assert_eq!(
            renamed("Piano/Grand/W.npno", "x", |_| false),
            "Piano/Grand/W [x].npno"
        );
        assert_eq!(renamed("W", "x", |_| false), "W [x]");
    }
}
