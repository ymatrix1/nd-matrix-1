# SMP page refill: descriptor law and flash feed (OQ41, D191)

Checked: 2026-11-21. Sources: SMP image listing `0x3818B4..0x381AB4` and `0x381B00..0x381CCA` (`target/oracle/smp.bin`, local), `.npno` files under `.reference/` (local only), `sharc-live --smp --epchain` runs of the four programs of [piano-chain-tap-smp](piano-chain-tap-smp.md), [smp-voice](smp-voice.md), [nsmp4-and-npno-cnsp-codec](nsmp4-and-npno-cnsp-codec.md).

Question: what does the SMP need so that the source ring of a voice holds the `.npno` page data, and does the piano chain tap then carry audio?

## 1. Short answer

| Part | Result | Tag |
|---|---|---|
| Descriptor to flash law | solved, see s2 | VERIFIED (run, 4 files) |
| Flash feed in `sharc-live --smp` | works: `L` loads a `.npno`, `F` places it, the page routine reads header and data and fills the stream rings with the file words | VERIFIED (run: ring head equals file payload, `tools/arm_smp_flash.py check_rings`) |
| Chain tap carries audio | **no.** After the first refill the voice phase and counter stop advancing, the tap is silent (grand, upright, EP) or a constant DC plateau (clavinet, 0.0431) | VERIFIED (run, `data/smp-page-refill.csv`) |
| `piano_no_chain_tap` | unchanged, 155 programs | |

## 2. The law

The ARM writes the stroke descriptor into the voice record: `rec[-11]` directory index of the file, `rec[-10]` page number of the stroke, `rec[-9]` frame count (`+0x0E` of the stroke record). The page routine (`0x1C0C25`, `0x1C0DA8`) computes

`R7 = DM(0xB237B + rec[-11] + (rec[-10] >> g)) + (rec[-10] & (2^g - 1)) * s`, `(g, s) = (6, 0x3FE)` stereo, `(7, 0x1FF)` mono.

Latch `R9 = R7 >> 24` goes to bank 2, `R7 & 0xFFFFFF` addresses bank 1 (page header word) and bank 3 (data). All banks are one 16-bit-word flash (listing, VERIFIED). `rec[-10]` of a stroke is `(file byte of the stroke + 148) / (1022 * channels)`, the file byte including the `0x2C` CNSP base (`tools/arm_smp_flash.py::stroke_page`). The ARM owns the flash file system, so the directory is a harness choice: `dir[i] = 65408 * i` (a group of 64 stereo or 128 mono pages) and `base = 65408 * rec[-11] + 74`.

Check: every descriptor seen in 4 runs (3 stereo files, 1 mono file) matches a stroke by frame count and `rec[-10] == stroke_page` (`law_ok` column of `data/smp-page-refill.csv`), and the ring head equals the file payload (`check_rings`).

## 3. The feed

`crates/nord-sharc/src/core/periph/smpflash.rs` serves banks 1 to 3 (core accesses and DMA chain reads), `runner/live_flash.rs` implements the protocol lines `L PATH` (load), `F REC11` (place and write the directory), `E` (report, `flash_b1`, `flash_b3`, `flash_outside`), `C PATH` (capture the tapped f32 frames), `H 0|1` (ring half). `tools/arm_smp_flash.py` watches the SMP messages, recognises the descriptor, and sends `F`. `arm_dsp_live.py --piano-bind BACKUP` attaches it and writes `smp_flash` into the result.

## 4. Why the tap stays silent (negative result, next blocker)

Replay of the key message script, record of voice 0 or 1 (words `-30..-27`) at passes 842 to 1464: the stereo strokes (rec11 28271) hold counter 0, phase 0, ring and gains set; the clavinet voice advances 3 counts (`0xC` to `9`) and then freezes. Meanwhile the DMA keeps running (chains 7 at pass 864, 89 at pass 1464) and the flash pointer `rec[-14]` flips between two values: the page is refilled again and again, the voice does not consume it. The tap of the clavinet is the DC value of the frozen sample (`0.04314` from frame 1500 to 3500).

Tried and ruled out: clearing `DMAC0.DEN` when the chain finishes (no change). Not ruled out: the voice scheduler gate (the words `0x9268A`, `0x92534` and the per-voice active test of the audio ISR), the completion interrupt of the EP DMA (not modelled, `epchain.rs`), the ARM re-init burst at pass 1470 (375 messages). Next step: trace which PC range runs per audio interrupt for a frozen voice (`sharc-live` `T` probes on the per-voice loop `0x1C07D8`) and compare with the oracle replay that advances the same record.

Also open: 129 `flash_outside` reads right after the key (a descriptor page past the placed file), layers 2 and 3 not separated, the stroke 523 type (flags 2, release) is the last descriptor in the rings.

## Reproduce

```
cargo build --release -p nord-sharc --bin sharc-live
python tools/arm_dsp_live.py ... --piano-bind BACKUP --key-ticks 3000   # smp_flash in out.json
python -m pytest tools/tests/test_arm_smp_flash.py
```

## Implications

- OQ41 blocker 3 is half closed: data reaches the ring bit-exactly; consumption does not happen.
- No ranking column, no change of `piano_no_chain_tap`.

## Follow-ups

1. Per-voice loop trace (s4), then the interrupt or flag that releases the voice.
2. Re-measure the four programs against the native peaks (-8.94, -16.97, -8.75, -5.98 dB) once the phase advances; then the census and `program-ranking.md`.
3. Separate piano layers 2 and 3.
