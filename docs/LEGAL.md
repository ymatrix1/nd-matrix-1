# Legal posture

- **Checked:** 2026-11-23
- **Status:** technical record of what the project does and does not do. **This is not legal advice** and not a legal opinion. It has not been reviewed by counsel. Where it names a statute it asks a question; it does not conclude.
- **Decision:** [D232](DECISIONS.md#d232-contributions-require-dco-sign-off-legal-posture-recorded-in-docslegalmd)

## 1. What the project does and why

ND Matrix 1 is a source-available, non-commercial emulator, DSP engine and Clavia Binary (`CBIN`, `.ns4b`, `.ns4p`) toolkit for the Nord Stage 4 (`product_id=46`). Its purpose is:

| Purpose | Meaning here |
|---|---|
| Interoperability | Read and write the file formats the hardware and the vendor's editor use, so other software and other keyboards can exchange programs, with lossless `CBIN` round trips. |
| Research | Measure how the hardware behaves (filters, envelopes, effects, organ and piano models) and record the measured numbers and equations. |
| Preservation | Keep the sound and the file formats usable after the hardware or the vendor's tools are gone. |

The engine is written from measurements and closed-form equations. Scope and non-goals are in [PRD.md](PRD.md).

## 2. How reference material is obtained

| Material | Source | Where it lives |
|---|---|---|
| OS update file (`.exe`, `.dmg`, `.zip`) | The vendor's public OS update download, or the user's own copy | Local, git-ignored `.reference/` |
| Factory or user backup (`.ns4b`) | The user's own keyboard, via the vendor's backup function | Local, git-ignored `.reference/` |
| Unpacked images, dumps, disassemblies | Produced locally from the files above | Local, git-ignored (`target/fw_unpacked/`, `.reference/`) |

None of these are committed. [D20](DECISIONS.md#d20-derived-values-only-no-firmware-bytes-in-the-repository) allows only derived values (addresses, field widths, small integers, closed-form equations, our own measurements). [D128](DECISIONS.md#d128-raw-firmware-artifacts-are-never-tracked-derived-data-is-classified-a-to-d-b-and-c-move-to-runtime-loading) forbids tracking raw artifacts (class A) and classifies every derived file A to D in [firmware-derived-data-inventory](research/firmware-derived-data-inventory.md). `.gitignore` blocks the installer and backup names. The app takes the user's own OS update file as input, see [os-update-input](research/os-update-input.md).

## 3. No circumvention

The evidence in this repository is that the firmware is not protected by encryption:

- [D12](DECISIONS.md#d12-pursue-track-b-reverse-engineering-for-cnsp-sample-codec-and-firmware-decompression) records that the `CODE` segment is unencrypted and compressed with a custom 16-bit LZ token scheme (`distance = token >> 3`, `length = (token & 7) + 2`).
- [1to1-emulation-feasibility](research/1to1-emulation-feasibility.md) section 9 ("Clavia Custom LZ Format Solved (No Encryption)") derives the token format by comparing the compressed stream with known ARM startup code. Section 1 of the same note records the earlier decision that no decryption or protection circumvention would be attempted.
- [ns4-firmware-teardown](research/ns4-firmware-teardown.md) section 4 lists the container layout. [nkr-resource-container](research/nkr-resource-container.md) records the same for the resource container ("Not encrypted").

The project does not bypass, remove or defeat any access control, signature check, encryption, licence check or other technical protection measure, and it does not accept contributions that do. Decompression is the only transformation applied to the update file.

## 4. What ships and what does not

| Ships | Does not ship |
|---|---|
| Source code, our own measurements, closed-form equations, test fixtures built in memory | Firmware or sample bytes, factory presets, `.ns4b` backups, installers |
| Original art and a neutral wordmark ("Art and branding" in [D16](DECISIONS.md#d16-phase-6b-scope-original-art-functional-or-stub-controls-ha88-first-additive-host-ids) | Nord logos, wordmarks, photographs or art, anything traced or sampled from them |
| Parsers and writers for the file formats | Disassembly listings of vendor code, unpacked images |

Users bring their own lawfully obtained update and backup files.

Found and removed: two Nord product photographs of the Stage 4 (`crates/nord-gui/assets/panel_texture.jpg`, `keyboard_full.jpg`, with the vendor logo) were tracked, and the first was embedded in every GUI binary. They contradicted D16 and are removed; `tools/check_no_vendor_photos.py` runs in CI to keep tracked JPEGs and `include_bytes!` of JPEGs out of `crates/` and `assets/`. They remain in git history (section 8, question 5).

Not yet fully true, stated plainly: the inventory still tracks class B (code images such as `bass-fw-image.csv`, `farfisa-fw-image.csv`, `organ/*/prog_*.rs`) and class C (numeric table dumps without a closed form such as `synth-p-words.csv`, `synth-fm-tables.csv`, `fx/fw_*_tables.rs`). D128 decides that these move to runtime loading from the user's own OS update file in follow-up PRs. Until each migration lands the files remain in the tree and in history. See [firmware-derived-data-inventory](research/firmware-derived-data-inventory.md) for the current list and order. Whether such tables are protectable at all, and in which jurisdiction, is an open question in section 8.

## 5. Trademarks

The product name is **ND Matrix 1**. "Nord", "Clavia" and "Nord Stage" are trademarks of their owners and appear only to describe what the software interoperates with. The project is not affiliated with or endorsed by Nord Keyboards / Clavia DMI AB (see the README Disclaimer).

## 6. Third-party references

Videos, courses, recordings and photos or models of the hardware are used only to measure behaviour (levels, timings, layout proportions, colour values). Nothing from them is redistributed: they sit in git-ignored `.reference/`, and only the measured numbers enter the repository.

- Art and branding, including photos and models as measurement-only reference: "Art and branding" in [D16](DECISIONS.md#d16-phase-6b-scope-original-art-functional-or-stub-controls-ha88-first-additive-host-ids).
- Course footage: [dsoundman-course-findings](research/dsoundman-course-findings.md) and [dsoundman-course-organ-piano](research/dsoundman-course-organ-piano.md) (sources "local, git-ignored").
- Video clip comparisons: [grand-pad-vs-video](research/grand-pad-vs-video.md), [b3-vox-video-gap](research/b3-vox-video-gap.md), [pipe-video-gap](research/pipe-video-gap.md), and D22.

## 7. Rights-holder contact and takedown

If you hold rights in material you believe is in this repository or its history:

1. Open a GitHub issue with the label `legal` using the "Rights-holder contact" template, naming the file, path or commit and what you ask for; or
2. Contact the maintainer through the GitHub profile [`ymatrix1`](https://github.com/ymatrix1).

We respond promptly. A module, table or file that is the subject of a good-faith request can be removed from the tree quickly, and we will discuss history. Do not post the disputed material itself in an issue; point to the path or commit.

## 8. Open legal questions for counsel

These are questions, not conclusions.

1. Does the vendor's OS updater licence (EULA) contain terms on reverse engineering, decompilation or use of the update file, and if so, how do they interact with the interoperability and research purposes above?
2. In the US, does 17 USC 1201(f) (reverse engineering for interoperability) cover this work, given that no technical protection measure is circumvented (section 3), and what does it say about publishing derived tables?
3. In the EU, does Art. 6 of the Software Directive 2009/24/EC (decompilation for interoperability) apply, and what limits does it put on publishing what is learned?
4. Which jurisdiction's rules apply to a project whose contributors and users are in several countries?
5. Commit `8a3f060` removed `helper.exe` (the vendor's USB driver installer), `sram.bin` and `sram.asm` from the tree, but they remain in git history, as do the two product photos `panel_texture.jpg` and `keyboard_full.jpg` (first tracked in or before commit `f821f0d7`, removed from the tree in the "remove vendor photos" change, section 4). Is history rewriting needed, and is that decision (left to the owner in D128) enough?
6. Are class B and C derived tables (section 4) copyrightable expression or unprotectable facts, and does moving them to runtime loading fully resolve the concern?
7. Does the PolyForm Noncommercial licence of this repository interact with any of the above?

## Implications

Every new constant, table or art asset has to fit sections 2 to 4. Contributions follow the DCO sign-off in [CONTRIBUTING.md](../CONTRIBUTING.md).

## Follow-ups

- Counsel review of section 8.
- Finish the class B and C migrations (D128) and update the inventory.
