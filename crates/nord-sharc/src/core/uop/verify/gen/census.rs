//! The valid instruction encodings of the committed census, per instruction kind. OWNER:
//! W1-VERIFY. Test-only (`include_str!` of a 1 MB case file must not sit in the library).
//!
//! The families lower one instruction kind each and need valid encodings of it. Writing opcodes by
//! hand is slow and wrong for the long tail (condition codes, the 48-bit forms), so they filter
//! this corpus instead: [`census_encodings`] returns every distinct instruction of the committed
//! census (`docs/research/sharc-seq/census_cases.txt` and `census_extra_cases.txt`), cases the
//! ADSP-21489 simulator ran, whose parcels the VISA classifier reads back as one whole
//! instruction of the kind it names.
//!
//! Provenance: the files are simulator-measured, not firmware-derived (D20, D128 are unaffected).
//! The corpus holds the encodings only (the first token of each case line); the state words of
//! the cases are the census replay's business (`verify/census.rs`).
use crate::visa::{classify, Kind};
use std::collections::BTreeSet;
use std::sync::OnceLock;

/// The committed census: the 1,119 cases of `census_cases.txt` and the 461 of
/// `census_extra_cases.txt` (together the 1,580 cases the design replays).
const CASES: [&str; 2] = [
    include_str!("../../../../../../../docs/research/sharc-seq/census_cases.txt"),
    include_str!("../../../../../../../docs/research/sharc-seq/census_extra_cases.txt"),
];

/// One distinct instruction of the census.
struct Encoding {
    kind: Kind,
    parcels: Vec<u16>,
}

/// The parcels of a hex token (four digits each), or `None` when it is not whole parcels.
fn parcels_of(token: &str) -> Option<Vec<u16>> {
    if token.is_empty() || !token.len().is_multiple_of(4) || token.len() > 12 {
        return None;
    }
    token
        .as_bytes()
        .chunks(4)
        .map(|d| u16::from_str_radix(std::str::from_utf8(d).ok()?, 16).ok())
        .collect()
}

/// Distinct instructions of the case files in file order, each with the kind and length the VISA
/// classifier gives its parcels. A token whose parcels are not one whole instruction (an unknown
/// kind, or a length other than the token's) is dropped.
fn parse(files: &[&str]) -> Vec<Encoding> {
    let mut seen = BTreeSet::new();
    files
        .iter()
        .flat_map(|f| f.lines())
        .filter_map(|l| l.split_whitespace().next())
        .filter(|t| t.bytes().all(|b| b.is_ascii_hexdigit()))
        .filter(|t| seen.insert(*t))
        .filter_map(parcels_of)
        .filter_map(|parcels| {
            let (kind, len) = classify(parcels[0], parcels.get(1).copied());
            (kind != Kind::Unknown && len == parcels.len()).then_some(Encoding { kind, parcels })
        })
        .collect()
}

fn corpus() -> &'static [Encoding] {
    static CORPUS: OnceLock<Vec<Encoding>> = OnceLock::new();
    CORPUS.get_or_init(|| parse(&CASES))
}

/// The census encodings for which `filter(kind, parcels)` holds, in file order. The result plugs
/// into `random_program_from(rng, &pool, n)` as the pool. Filter on the kind
/// (`|k, _| k == Kind::T1a`) or on the parcels (a compute field, a condition code).
pub fn census_encodings(filter: impl Fn(Kind, &[u16]) -> bool) -> Vec<&'static [u16]> {
    corpus()
        .iter()
        .filter(|e| filter(e.kind, &e.parcels))
        .map(|e| e.parcels.as_slice())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::lower::decode::decode_at;
    use crate::core::uop::verify::gen::{core_with_code, random_program_from, Rng};

    #[test]
    fn the_corpus_is_the_distinct_valid_instructions_of_the_committed_file() {
        let all = census_encodings(|_, _| true);
        let tokens: BTreeSet<_> = CASES
            .iter()
            .flat_map(|f| f.lines())
            .filter_map(|l| l.split_whitespace().next())
            .filter(|t| t.bytes().all(|b| b.is_ascii_hexdigit()))
            .collect();
        assert!(all.len() > 250, "{} encodings", all.len());
        assert!(
            all.len() <= tokens.len(),
            "distinct tokens bound the corpus"
        );
        let distinct: BTreeSet<_> = all.iter().collect();
        assert_eq!(distinct.len(), all.len(), "no encoding twice");
        assert_eq!(
            all,
            census_encodings(|_, _| true),
            "a second call gives the same list"
        );
    }

    #[test]
    fn every_encoding_decodes_to_one_whole_instruction_of_its_kind() {
        let mut kinds = BTreeSet::new();
        for e in corpus() {
            let mut c = core_with_code(&[&e.parcels[..], &[0x0001]].concat());
            let pc = c.pc;
            let d = decode_at(&mut c, pc);
            assert_eq!(
                (d.insn.kind, d.insn.len, d.next_pc - d.pc),
                (e.kind, e.parcels.len(), e.parcels.len() as u32),
                "{:x?}",
                e.parcels
            );
            kinds.insert(e.kind.name());
        }
        // the census spans the instruction types the families lower, 16, 32 and 48 bits
        for kind in ["1a", "2a", "3a", "4a", "5a", "7a", "14a", "17a", "17b"] {
            assert!(kinds.contains(kind), "no {kind} in {kinds:?}");
        }
        let mut per_kind = std::collections::BTreeMap::<&str, usize>::new();
        for e in corpus() {
            *per_kind.entry(e.kind.name()).or_insert(0) += 1;
        }
        // VERIFIED: the counts below are what `parse` reads from the two committed case files
        // (this test; a change to the census files must update them on purpose)
        let want = [
            ("14a", 135),
            ("15b", 4),
            ("16a", 1),
            ("17a", 4),
            ("17b", 1),
            ("1a", 18),
            ("2a", 23),
            ("2b", 11),
            ("3a", 39),
            ("3b", 9),
            ("4a", 14),
            ("4b", 26),
            ("5a", 63),
            ("5b", 23),
            ("6n", 99),
            ("7a", 2),
        ];
        assert_eq!(per_kind.into_iter().collect::<Vec<_>>(), want);
        assert_eq!(corpus().len(), 472);
        let lens: BTreeSet<_> = corpus().iter().map(|e| e.parcels.len()).collect();
        assert_eq!(lens.into_iter().collect::<Vec<_>>(), [2, 3]);
    }

    #[test]
    fn a_filter_selects_by_kind_or_by_parcels() {
        let t1a = census_encodings(|k, _| k == Kind::T1a);
        assert!(t1a.len() > 5);
        assert!(t1a.iter().all(|p| p.len() == 3));
        let by_parcel = census_encodings(|_, p| p[0] == t1a[0][0]);
        assert!(by_parcel.contains(&t1a[0]));
        assert!(census_encodings(|_, _| false).is_empty());
        let (two, all) = (
            census_encodings(|_, p| p.len() == 2),
            census_encodings(|_, _| true),
        );
        assert!(!two.is_empty() && two.len() < all.len());
    }

    #[test]
    fn programs_drawn_from_a_filtered_corpus_stay_whole_instructions() {
        let pool = census_encodings(|k, _| matches!(k, Kind::T1a | Kind::T2a | Kind::T9a));
        assert!(pool.len() > 10);
        let prog = random_program_from(&mut Rng::new(4), &pool, 50);
        let mut c = core_with_code(&prog);
        let (mut pc, end, mut n) = (c.pc, c.pc + prog.len() as u32, 0);
        while pc < end {
            pc = decode_at(&mut c, pc).next_pc;
            n += 1;
        }
        assert_eq!((pc, n), (end, 50));
    }

    #[test]
    fn the_parser_drops_state_words_unknown_kinds_and_repeats_across_files() {
        // `scr` line, a repeated instruction, a state-sized token, a token of half a parcel
        let one = "scr 11 22
0001 00000000
0001 55555555
7FFFFFFF 1
123
01863001 1 2
";
        let two = "0001 9
7000813F 1
";
        let got = parse(&[one, two]);
        assert_eq!(
            got.iter().map(|e| e.parcels.clone()).collect::<Vec<_>>(),
            [vec![0x0001], vec![0x0186, 0x3001], vec![0x7000, 0x813F]]
        );
        assert_eq!(parcels_of("0186300"), None);
        assert_eq!(parcels_of(""), None);
        assert_eq!(parcels_of("0001"), Some(vec![1]));
    }
}
