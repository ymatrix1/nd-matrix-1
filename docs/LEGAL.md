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

### Updater terms check (v1.66)

On 2026-11-23 `Update Nord Stage 4 OS v1.66.zip` was downloaded from the vendor's public download link (document id 1042 in `tools/nord_os_urls.py`, which redirects to the vendor's asset host) into git-ignored `.reference/`. `ns4 firmware info` confirmed the container: 4,037,080 bytes, fnv1a64 `5fd75f195e3671be`, 46 WAVE tables (matches [os-update-input](research/os-update-input.md)).

A text search of the `.exe` inside it found no end-user licence agreement and no reverse-engineering or decompilation restriction:

```bash
F=".reference/os166/Update Nord Stage 4 OS v1.66.exe"
for o in -a -el; do strings $o "$F" | grep -iE "license agreement|reverse engineer|decompil|disassembl|all rights reserved"; done
```

The only match is the Microsoft C runtime line "Copyright (c) by P.J. Plauger, licensed by Dinkumware, Ltd. ALL RIGHTS RESERVED." It is not a Nord term.

### Updater terms check: macOS `.dmg` (v1.66)

On 2026-11-23 the local, git-ignored `Update Nord Stage 4 OS v1.66.dmg` (13,330,920 bytes, sha256 prefix `16dd08b9f47ad2ea`) was inspected. It was extracted with 7-Zip into a new empty scratch directory outside the repository; nothing in it was executed or committed.

| Check | Result |
|---|---|
| DMG metadata (`koly` trailer, XML plist at offset 13,318,816) | Resource keys `blkx` and `plst` only. No licence-agreement resource (`sla `, `LPic`, `STR#`, `TEXT`, `RTF`) |
| Volume contents (HFS+) | `.app` bundle, `bg-updater.tiff`, `.DS_Store`, `.fseventsd`. No licence, readme or terms file |
| `Contents/Info.plist`, `_CodeSignature/CodeResources` | No licence or terms entry. Bundle id `com.nordkeyboards.NordOSUpdate` |
| `strings -a` and `strings -el` on `Contents/MacOS/Nord OS Update` (22,076,656 bytes) | No match for `licen[sc]e`, `eula`, `agreement`, `reverse engineer`, `decompil`, `disassembl`, `all rights reserved`. Only third-party copyright lines (zlib, libjpeg, libpng) and a code-signing certificate line about "standard terms and conditions of use" |
| `strings -a` on `Contents/Resources/os.cab` (4,037,080 bytes, the same size as the Windows update container) | 0 matches for `reverse engineer`, `decompil`, `eula`, `license agreement` |

Result: the macOS updater, like the Windows `.exe`, shows no EULA text and no reverse-engineering or decompilation restriction in any file or string of the `.dmg`.

### Vendor website check (nordkeyboards.com)

On 2026-11-24 the following pages were read in a browser without logging in, accepting cookies or terms, or downloading any file:

| Page | Finding |
|---|---|
| `https://www.nordkeyboards.com/downloads/` | Product picker only. No licence or terms text |
| `https://www.nordkeyboards.com/products/nord-stage-4/downloads` | Lists "Update Nord Stage 4 OS v1.66.dmg" (macOS) and ".zip" (Windows), both dated 2026-08-25, with links to the update instructions PDF, manuals, factory restore files and the USB driver. No licence, terms or reverse-engineering text on the page |
| `https://www.nordkeyboards.com/support/` | Warranty goes through distributors. No terms of use |
| `https://www.nordkeyboards.com/support/privacy-policy/` | Privacy policy only. No match for terms of use, licence, reverse engineering, decompilation or copyright |
| `/about-us/`, `/support/feedback/`, `/software/`, `/` | No match for the same terms |
| Footer (every page) | Links: Products, Sounds, Downloads, Software, Artists, About Us, Distributors, Support, News, social links, "Cookie settings". Line: "Copyright © 2026 Clavia DMI AB. All rights reserved." There is no "Terms of use" or "Legal" link |
| `/terms/`, `/terms-of-use/`, `/terms-and-conditions/`, `/legal/`, `/support/terms/`, `/support/terms-of-use/`, `/support/legal/`, `/support/license/` | HTTP 404 |

The project now links the Stage 4 Downloads page ([guide](guides/nord-os-update.md), `firmware_setting::DOWNLOADS_PAGE`) instead of direct `.zip` and `.dmg` file links, so the user downloads from the vendor's own page. This is a factual record, not a legal conclusion.

Limits:

- Only the v1.66 `.exe` and `.dmg` were checked. Older OS versions were not.
- A string search cannot prove the absence of terms shown at runtime (for example in an installer dialog built from resources `strings` does not decode) or behind a download dialog. The updaters were not run.
- The website was read on one day and may change. Only the pages above were read. Terms shown only to logged-in users or in a cookie or download dialog were not seen.

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

**Musicians.** PolyForm Noncommercial 1.0.0 counts personal use as permitted only "without any anticipated commercial application", so a paid gig or a sold recording is arguably outside it. The licensor grants an additional permission in [ADDITIONAL-PERMISSIONS.md](../ADDITIONAL-PERMISSIONS.md): making, performing, recording, broadcasting or streaming music, paid or not, is a permitted purpose; selling, renting, sublicensing, bundling or distributing the software for a fee or in a commercial product or service is not. The `LICENSE` text is unchanged (D241). The wording is a draft and has not been reviewed by counsel (`ns4-ns2j`). This is not legal advice.

Every release archive and installer ships four license documents. Two are new: [ADDITIONAL-PERMISSIONS.md](../ADDITIONAL-PERMISSIONS.md) and [TRADEMARKS.md](../TRADEMARKS.md) ([D241](DECISIONS.md#d241-licensor-additional-permission-for-musicians-and-a-trademark-policy-ship-as-separate-files)). The others are our `LICENSE` (PolyForm Noncommercial 1.0.0, D231) and `THIRD_PARTY_LICENSES.html`, the copyright and license texts of the third-party crates in the binaries (D238). The notices are generated by `cargo-about` at release time from `Cargo.lock`, `about.toml` and `about.hbs`, cover the vendored `nih_plug_egui` (ISC) and the fonts of `epaint_default_fonts` (MIT, OFL-1.1, Ubuntu-font-1.0), and are checked by `tools/release_verify.py`. CI job `third-party-notices` fails when a crate license is not on the accepted list. The `.AppImage` also contains a runtime that `appimagetool` embeds, which `THIRD_PARTY_LICENSES.html` does not cover. Release builds pin it to AppImage type2-runtime `20251108` (sha256 `2fca8b44...ec260d`, [D242](DECISIONS.md#d242-the-appimage-runtime-is-pinned-and-its-notices-ship-with-the-linux-release)); its notices are in [licenses/appimage-runtime/](../licenses/appimage-runtime/README.md), shipped in the AppImage (`usr/share/doc/nd-matrix-1/appimage-runtime/`) and in the Linux archive, and checked by `tools/release_verify.py`.

| Runtime component | Version | License | Linked |
|---|---|---|---|
| type2-runtime (`runtime.c`) | `20251108` | MIT | the program itself |
| musl libc | Alpine 3.21 package | MIT | static |
| libfuse (patched: `fusermount3` path from the environment) | 3.15.0 | **LGPL-2.1** | static |
| squashfuse | 0.5.2 | BSD-2-Clause | static |
| zstd | 1.5.6 | BSD-3-Clause (of BSD or GPL-2.0) | static |
| zlib | 1.3.1 | zlib | static |
| mimalloc | Alpine 3.21 package, version not verified | MIT | static |

Evidence: the runtime source at tag `20251108` (`LICENSE` names musl, libfuse, squashfuse, libzstd and zlib; `src/runtime/Makefile` links `-lsquashfuse -lsquashfuse_ll -lzstd -lz -lfuse3 -lmimalloc` with `-static-pie`; `scripts/common/install-dependencies.sh` builds libfuse 3.15.0 and squashfuse 0.5.2 from sha256-pinned tarballs) and `strings` on the shipped `runtime-x86_64` (`squashfuse 0.5.2`, `3.15.0`, `1.5.6`, `inflate 1.3.1`, mimalloc messages). Not verified: the musl and mimalloc package versions (`gitlab.alpinelinux.org` is blocked from the build environment). `git.musl-libc.org` was blocked too, so the musl text is from the `kraj/musl` GitHub mirror at tag `v1.2.5`.

**LGPL-2.1 obligation (resolved by option A, pending counsel confirmation).** libfuse is statically linked into the runtime, so the runtime executable is a combined work under LGPL-2.1 section 6. Whoever distributes the `.AppImage` must (a) carry a notice that libfuse is used and a copy of the LGPL, (b) let the recipient modify the combined work and relink it, and (c) deliver the corresponding source with the work, give a written offer, or, when distributing from a place, offer the same access to the source from the same place (6a, 6c, 6d). The owner chose to ship the source: every release carries the asset `nd-matrix-1-<tag>-appimage-runtime-sources.tar.gz` next to the `.AppImage` (same release page, so 6d is met in place), built by `tools/release_runtime_sources.py` in `release.yml`. It holds libfuse 3.15.0 (sha256 `70589cfd...ea2ca0`), squashfuse 0.5.2 (sha256 `db0238c5...9f443d3`), the type2-runtime source at commit `dd6cebed...e43868` (`git archive`; it holds the build scripts and `BUILD.md`) and the libfuse patch, plus `SOURCES.txt` with the relink steps (rebuild the runtime from that source with a modified libfuse, repack with `appimagetool --runtime-file`). The script fails the release when a download's sha256 or the runtime tag's commit differs from the pin. The libfuse hash was computed here from the downloaded file; the squashfuse hash is the one the runtime's own `install-dependencies.sh` checks, and its download was blocked from the machine that wrote this, so the first release run is its first end-to-end check. `tools/release_verify.py` fails a Linux release without the asset, with a missing member or without its `.sha256`. The notice and the license text ship in the AppImage and the archive (`licenses/appimage-runtime/README.md`). The application inside the SquashFS image is data and is not linked with libfuse, so its license should be unaffected. Whether this meets section 6 is for counsel to confirm (bean `ns4-ns2j`). Alternatives the owner did not choose: a runtime that loads libfuse dynamically (no such upstream build known to us, unverified), a different or self-built runtime, dropping the `.AppImage`. Tracked in bean `ns4-sgro`.

Not yet fully true, stated plainly: the inventory still tracks class B (code images and generated translations such as `vox_image.rs` and `firmware_bank.rs`; the Farfisa and B3 Bass images and rows are built at run time since 2026-11-23 and 2026-11-24) and class C (numeric table dumps without a closed form such as `synth-p-words.csv`, `synth-fm-tables.csv`, `fx/fw_*_tables.rs`). D128 decides that these move to runtime loading from the user's own OS update file in follow-up PRs, prioritizing class B first. Until each migration lands the files remain in the tree and in history. See [firmware-derived-data-inventory](research/firmware-derived-data-inventory.md) for the current list and order. Whether such tables are protectable at all, and in which jurisdiction, is an open question in section 8.

## 5. Trademarks

The name "ND Matrix 1" and its icon belong to the project; forks and modified builds must use a different name and icon, and descriptive use ("based on ND Matrix 1") is fine. The policy is [TRADEMARKS.md](../TRADEMARKS.md) (D241). The name is not registered; whether to register it (jurisdiction, cost) is an open owner decision (`ns4-0s8y`).

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

We respond promptly (target: acknowledge within 48 hours). A module, table, release asset or file that is the subject of a good-faith request will be removed or disabled quickly, and we will work with the rights holder on history or archiving if needed. Do not post the disputed material itself in an issue; point to the path or commit.

## 8. Open legal questions for counsel

These are questions, not conclusions.

1. Does the vendor's OS updater licence (EULA) contain terms on reverse engineering, decompilation or use of the update file, and if so, how do they interact with the interoperability and research purposes above? Findings so far (section 2): a string search of the v1.66 `.exe` (2026-11-23) and an inspection of the v1.66 `.dmg` (2026-11-23: no licence resource, no EULA or reverse-engineering strings) found no EULA and no such restriction; the vendor's Downloads pages and site (2026-11-24) show no terms of use and no licence text, only a footer copyright line. Other OS versions and runtime-only dialogs were not checked.
2. In the US, does 17 USC 1201(f) (reverse engineering for interoperability) cover this work, given that no technical protection measure is circumvented (section 3), and what does it say about publishing derived tables?
3. In the EU, does Art. 6 of the Software Directive 2009/24/EC (decompilation for interoperability) apply, and what limits does it put on publishing what is learned?
4. Which jurisdiction's rules apply to a project whose contributors and users are in several countries?
5. History was rewritten on 2026-11-23 (owner decision, `git filter-repo --invert-paths`): `helper.exe`, `sram.bin`, `sram.asm`, `crates/nord-gui/assets/panel_texture.jpg`, `crates/nord-gui/assets/keyboard_full.jpg`, `docs/img/gui/after-overview.png`, `docs/research/data/farfisa-fw-image.csv`, `crates/nord-core/src/organ/farfisa_fw/prog_proc.rs` and `crates/nord-core/src/organ/b3_bass_fw/native_farfisa.rs` are gone from every branch; the file tree of `main` was unchanged by the rewrite. Release tags `v0.1.0` to `v0.1.2` have been moved on `origin` to the rewritten commits. Still open: read-only pull request refs on GitHub keep the old commits reachable until GitHub Support purges them; forks and earlier clones keep their copies; class B and C files still on `main` stay in history until they are migrated, after which one more rewrite is planned. Is this enough, or should further steps be taken?
6. Are class B and C derived tables (section 4) copyrightable expression or unprotectable facts, and does moving them to runtime loading fully resolve the concern?
7. Does the PolyForm Noncommercial licence of this repository interact with any of the above?
8. Do any vendor or third-party patents cover features we emulate (organ, piano, synth or effect algorithms, file formats), and does interoperability or research use give any defence? No patent search has been done.
9. Answered, pending counsel confirmation. The AppImage runtime (type2-runtime `20251108`) is MIT and statically links musl (MIT), libfuse 3.15.0 (LGPL-2.1), squashfuse 0.5.2 (BSD-2-Clause), zstd 1.5.6 (BSD-3-Clause), zlib 1.3.1 (zlib) and mimalloc (MIT). The notices ship, and the LGPL-2.1 step is met by option A: the corresponding source and the relink steps are a release asset beside the AppImage (section 4, D242). Question for counsel: does that satisfy LGPL-2.1 section 6, and is the application payload, a separate work in the SquashFS image, unaffected?

## 9. Patents and the project licence

The PolyForm Noncommercial 1.0.0 licence ([LICENSE](../LICENSE)) contains a Patent License and a Patent Defense clause. The Patent License grants users the licensor's own patent claims that they would infringe by using the software. The Patent Defense clause ends that grant if a user claims in writing that the software infringes a patent. Neither covers anyone else's patents: the licence cannot grant rights in a vendor's or third party's patents, and it gives no defence against them. The licensor knows of no patents of its own that read on the software. No patent search has been done (section 8, question 8).

## Implications

Every new constant, table or art asset has to fit sections 2 to 4. Contributions follow the DCO sign-off in [CONTRIBUTING.md](../CONTRIBUTING.md).

## Follow-ups ($0 legal budget posture)

- **Tier 1 (quick wins):** Remove in-GUI sponsorship button (`ns4-s3pn`), GitHub Support purge of old PR refs (`ns4-vt4r`), required checks (`ns4-6fbu`), and free `.dmg`/website terms check (`ns4-d7mg`). (`ns4-sgro` AppImage runtime notices and LGPL source asset shipped in D242; `ns4-b9rn` renamed the standalone binary to `nd-matrix-1-standalone` and `release_verify.py` now rejects any shipped `nord-` name.)
- **Tier 2 (high-impact engineering):** Finish class B (first) and class C migrations (`ns4-u8o2`, D128), run the second history rewrite (`ns4-eh57`), and ship a neutral non-red default panel theme (`ns4-80xe`; done, D246: original Slate Matrix identity, red look deleted).
- **Decision recorded:** Keep DCO without a CLA (`ns4-9ngp`); `ADDITIONAL-PERMISSIONS.md` covers `ymatrix1 and each contributor to ND Matrix 1`.
- **Deferred / optional:** Paid counsel review of section 8 (`ns4-ns2j`) and paid trademark registration (`ns4-0s8y`) are deferred unless the project ever commercializes or receives a formal legal notice.
