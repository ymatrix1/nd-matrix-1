# LLE live I/O (engine-neutral half of `ns4-lle6`)

Status: v1.0, 2026-11-23. Code: `crates/nord-core/src/lle_io/`. Epic `ns4-lle1`, bean `ns4-lle6`.

Everything the future `LleEngine` (`ns4-lle5`) needs to be played and watched, with no dependency on ARM0 (`ns4-lle2`) or SHARC (`ns4-lle3`). Wiring into ARM0 input queues stays in `ns4-lle6` after `ns4-lle5`.

## Data flow

```
MIDI callback / GUI --midi_bytes / panel_edit--> LleEvent --EventProducer--> ring --EventConsumer--> LleInput::drain --> LleEngine (later: ARM0 queues)
GUI <--TelemetryReader::read-- seqlock <--TelemetryWriter::publish-- LleTelemetry::fill_telemetry <-- LleEngine (OLED RAM, LEDs)
```

`lle_io::link(capacity)` builds both ends once (the only allocation); all hot calls are allocation- and lock-free.

## Modules

| File | Content |
|---|---|
| `ring.rs` | `channel::<T: Wire>(cap)`: wait-free SPSC ring, slots `[AtomicU64; 2]` (no `unsafe`), `Producer::push` returns `false` and counts a drop when full (the newest event is lost, even a `NoteOff`), `Consumer::pop`, `Consumer::take_new_drops` (drops since the last call). Capacity rounds up to a power of two. `Producer` / `Consumer` are `&mut self`, not `Clone`: SPSC is a compile-time property. |
| `seqlock.rs` | `SeqCell<const W>`: single-writer seqlock over `AtomicU32` words, same scheme as `engine::telemetry_cell` ([telemetry](telemetry.md)); reader retries up to 64 times and **may return `false` under load** (keep the previous snapshot, retry next frame). |
| `event/mod.rs`, `event/wire.rs` | `LleEvent` (16-byte wire form): `NoteOn/NoteOff`, `ControlChange`, `ChannelAftertouch` (channel pressure only: polyphonic aftertouch `0xA0` is not parsed by `MidiEvent::from_bytes` and is dropped), `PitchBend` (14 bit), `ProgramChange`, `Pedal{Sustain,Sostenuto,Soft,Control}`, `Panel(PanelEvent)`, `Nop`. `From<MidiEvent>` reuses `midi::MidiEvent` and turns CC 64 / 66 / 67 / 11 into `Pedal`. `PanelEvent`: `Encoder{index, delta}`, `Key{id, pressed}`, `Analog{id, value: u16}`. |
| `translate.rs` | `midi_bytes(&[u8])` (raw bytes to event), `panel_edit(PanelKind, id, value)` (GUI edit to event; `None` when a `Button` / `Encoder` id exceeds 255, so it never aliases another control), `unit_to_u16`. `PanelKind` mirrors `nord_panel::ControlKind` without a crate dependency. |
| `telemetry.rs` | `TelemetrySnapshot { oled: [[u8; 1024]; 2], leds: [u32; 16], frame, arm_ms }`; `channel()` gives a non-`Clone` `TelemetryWriter` and a `Clone` `TelemetryReader`; `read(&mut out)` fills a caller buffer. |
| `mod.rs` | `link`, traits `LleInput` (`handle_event`, bounded `drain`, at most 256 events per call; after the queue empties, `resync_after_drop` sends all-notes-off and sustain / sostenuto / soft up on 16 channels when the ring lost events) and `LleTelemetry` (`fill_telemetry`, `publish_telemetry`). |

## Provenance

| Item | Tag | Evidence |
|---|---|---|
| Two 128 x 64 OLEDs, 1024-byte framebuffer | `VERIFIED` | [arm-oled-render](research/arm-oled-render.md), `tools/arm_panel.py capture` |
| Encoder index / key id as firmware UI event ids | `VERIFIED` | `tools/arm_events.py` (`turn`), research note s15; not covered by a Rust test |
| Framebuffer layout: byte `page * 128 + x`, bit `i` = row `page * 8 + i` | `VERIFIED` | flush `0xC0076924` and the draw routines, [arm-oled-render](research/arm-oled-render.md) s2.3, `tools/arm_panel.py capture` (the Rust test only proves our accessor matches it) |
| Glass shows RAM upright | `ASSUMED` | same note s2.3; a video frame would confirm |
| CC 64 / 66 / 67 / 11 to `Pedal` (CC 11 is Expression in standard MIDI, named control pedal by `midi::cc`) | `ASSUMED` | confirm against the firmware CC table with `ns4-lle5` |
| 512 LEDs (`LED_WORDS = 16`), analog `u16` scale, `MAX_EVENTS_PER_DRAIN = 256`, 64 reader retries | `ASSUMED` | sizes only; real LED map and ADC scale come with `ns4-lle5` |

## Tests

| Test | Proves |
|---|---|
| `ring::tests::concurrent_producer_consumer_preserves_order` | 200 000 items across two threads, in order, no loss, no torn slot |
| `ring::tests::*` | FIFO, full ring rejects and counts drops, `take_new_drops` reports each loss once, wrap-around |
| `seqlock::tests::concurrent_readers_never_see_a_torn_snapshot`, `telemetry::tests::concurrent_readers_never_see_a_torn_snapshot` | 1 writer, 3 readers: every snapshot consistent |
| `event::tests::*`, `event::wire::tests::*` | wire round trip of every variant, unknown event or pedal tag to `Nop`, pedal CCs, every `MidiEvent` kind |
| `translate::tests::*` | raw MIDI, rejected system / truncated messages, panel mapping per kind, clamping, ids above 255 rejected for Button / Encoder (regression: `id as u8` aliased 277 to key 21) |
| `lle_io::tests::*` | end to end host to engine, bounded drain, one resync per ring overflow, telemetry back to host |
| `tests/no_alloc_lle_io.rs` | counting `#[global_allocator]`: 1000 rounds of translate, push, pop, publish, read make **0 allocator calls** |

## Not covered (blocked on `ns4-lle5`)

Applying events to the ARM0 input queues, the real `.ns4b` program store (`ns4-pyqg`), real OLED / LED data from the engine, and the GUI side polling `TelemetryReader` are not done here.
