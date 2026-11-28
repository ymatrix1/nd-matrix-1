# OS update file as the app's input

- **Checked:** 2026-11-17
- **Sources:** local files `Update Nord Stage 4 OS v1.66.exe`, `.dmg` and the `v1.44` to `v1.64` `.zip` files in `.reference/` (not committed); `target/fw_unpacked/fw_v1.66.bin` (the container unpacked earlier); [ns4-firmware-teardown](ns4-firmware-teardown.md)
- **Question:** can the app take the vendor's update file directly, for each format Nord ships?

## 1. Results (v1.66, real files; numbers only)

Command: `ns4 firmware info <file>` (`firmware_setting::info`: container size, FNV-1a 64 fingerprint, WAVE table count).

| Input | Container bytes | fnv1a64 | WAVE tables |
|---|---|---|---|
| `Update Nord Stage 4 OS v1.66.exe` (12,770,968 B) | 4,037,080 | `5fd75f195e3671be` | 46 |
| `Update Nord Stage 4 OS v1.66.dmg` (13,330,920 B) | 4,037,080 | `5fd75f195e3671be` | 46 |
| `fw_v1.66.bin` (unpacked earlier) | 4,037,080 | `5fd75f195e3671be` | 46 |

An independent Python FNV-1a over `fw_v1.66.bin` gives the same `5fd75f195e3671be`. In the `.exe` the container sits at file offset `0x79CB30` (RCDATA 101); the sum check `0x20 + 0x368F86 + 0x70A32 == 0x3D99D8` holds. RCDATA 105 (USB driver installer) has no container header and is never matched.

Zips `v1.44, 1.46, 1.52, 1.54, 1.60, 1.62, 1.64`: each holds one `.exe` (deflate); the container is extracted, then `ns4 firmware info` stops with "unsupported Nord OS version, WAVE registry not found (registry record 0 has a foreign layout)", because the registry address `0xC02FD4B0` is the v1.66 one.

## 2. The `.dmg` verdict

The `.dmg` is an Apple UDIF image: 512-byte `koly` trailer, XML plist with `mish` block tables for a GPT map (one `Apple_HFS` partition), 35 chunks of type `0x80000005` (zlib) plus zero chunks. **No filesystem reader is needed:** after decoding the chunks into a flat 41,943,040 B image, the container header occurs exactly once (offset `0x1CC1000`) and the HFS+ file is contiguous, so a header scan with the size-sum check returns the container, byte-identical to `fw_v1.66.bin`. ADC and bzip2 chunk types are rejected with a clear message (not present in v1.66). An updater that stores the container fragmented or compressed would not be found; the message is then "this is not a Nord OS update".

## Implications

- Terms check: a string search of the v1.66 `.exe` found no EULA or reverse-engineering clause; limits in [LEGAL](../LEGAL.md) section 2.
- The user needs one download and no unpacking tool (D129).
- Reading costs one pass over the file at each start (`.dmg`: zlib decode of 40 MB).

## Follow-ups

- Later OS versions: done (2026-11-27, [os-1-68-vs-1-66](os-1-68-vs-1-66.md)): the registry is found by its layout (v1.66 address first), so `ns4 firmware info` reports 46 WAVE tables for every update from v1.44 to v1.68. The first guess `0xC02FD4B0` is v1.66; v1.68 has it at `0xC02FD850`.
- A native file dialog (`rfd`) if drag-and-drop proves awkward.
