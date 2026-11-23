#!/usr/bin/env python3
r"""Run the booted ARM firmware against several live emulated DSP chips (`sharc-live`, D138, D144).

The ARM harness (`arm_screens.Session`, the full machine of `arm_dsp_capture`) boots with one
factory program in the Store record. Every ARM -> DSP message is grouped as the firmware stores it
(`arm_dsp_link.PortGrouper` on the real EMIFA ports during the boot, on the fake ports after it);
`arm_dsp_route.Router` sends each message to the chip that owns its `(comm, core, path)`, the rest
are only counted. After the boot ONE clock (24 kHz ARM tick, `arm_dsp_link.Clock`) runs every chip:
after each tick the owed passes run on all of them. Phases: `load` (program recall), `idle`, `key`
(one note held), `off` (released). Numbers only on stdout (JSON); each chip's sent messages go to
`--log DIR` as `sharc-sys --msg-script` files so a replay of the same stream can be compared.

    C=target/fw_unpacked
    uv run --with unicorn python tools/arm_dsp_live.py $C/bank0_CODE.bin $C/bank0_SRAM.bin \
        --eeprom target/oled/eeprom.bin \
        --program "factory_programs/Program/Bank A/Smooth Lead.ns4p" \
        --chip "ORG:2=org=ORG/core2_sram.bin=0xC1F83:4:2==0xC1174,0xC1180,0xC118E" \
        --chip "PNO:2=org=target/PNO/core2_sram.bin=0xC1F83:4:2==0xC1174,0xC1180,0xC118E" \
        --live target/release/sharc-live --sys-exe target/release/sharc-sys \
        --log live --out live.json

Chip spec `CHIP:CORE=PRESET=IMAGE[=WINDOW[=PROBES[=WATCH]]]` (`arm_dsp_route.parse_chip_spec`);
the SY chip is `SY:0=sy=IMAGE` (path 1, interrupt driven pass, D153; WINDOW `epchain` adds the
EP DMA model). The
old one-chip flags (`--dsp-image`, `--probe`, `--flags`) still build `PNO:1=pno=IMAGE`.

The ARM side keeps the harness conventions: 200,000 instructions per timer tick (16 x the hardware
budget, `arm-tick-timers.md` s5), `arm_sy_voice.pump` after every tick, `DspAck` set.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
import zipfile
from pathlib import Path

import arm_dsp_link as dl
import arm_dsp_return as rtn
import arm_dsp_route as rt
import arm_sy_timeline as syt
from arm_dsp_live_parts import (  # noqa: F401  (re-exported)
    Chips,
    Feeder,
    phase_delta,
    progress,
    return_observer,
    return_report,
    return_source,
    seen_delta,
    snapshot,
)

LOAD_TICKS, IDLE_TICKS, KEY_TICKS, OFF_TICKS = 800, 100, 1500, 300
KEY, VELOCITY = 60, 100
BOOT_CHUNK = 50_000_000  # as `Session.boot`


def boot_live(session, feeder: Feeder, chips: Chips) -> tuple[int, int]:  # noqa: ANN001  # pragma: no cover
    """`Session.boot` with the messages fed after every chunk; returns `(pc, instructions)`."""
    import arm_boot as ab
    import arm_emu as ae
    import arm_panel as ap
    import arm_steady as st

    emu = session.emu
    emu.set_reg(13, ae.STACK_TOP - 0x1000)
    emu.set_reg(14, ae.SENTINEL)
    pc, total = ab.APP_THUNK, 0
    while total < 1_500_000_000:
        pc, ran, err = st.run_until(emu, st.MAIN_LOOP, BOOT_CHUNK, pc=pc)
        total += ran
        feeder.feed()
        sent = {s.name: link.stats.messages for s, link in chips.items}
        progress(f"boot {total // 1_000_000} M insns pc={pc:#x} to chips {sent}")
        if pc == st.MAIN_LOOP or err:
            break
        if ap.in_wait_loop(pc):
            st.inject_irq(emu)
    st.enable_real_timer(emu)
    return pc, total


def run_ticks(rig, feeder: Feeder, clock: dl.Clock, chips: Chips, ticks: int, insns: int,  # noqa: ANN001  # pragma: no cover
              events: dict | None = None, probe=None, log=None) -> None:  # fmt: skip
    """ARM tick by tick: instructions, queue pump, messages to the chips, then the owed passes.

    `events` maps a tick of this phase to an action run first (the chord notes); `probe`
    (`arm_sy_timeline.SyProbe`) takes one SY sample per tick (D153).
    """
    import arm_steady as ast
    import arm_sy_voice as sv

    for n in range(ticks):
        if n % 200 == 0:
            progress(f"tick {n}/{ticks} passes {chips.lead.pass_index}")
        if events and n in events:
            events[n]()
        feeder.tick = clock.ticks
        if log is not None:
            log.tick = clock.ticks
        ast.run_ticking(rig.emu, insns, per_tick=insns)
        sv.pump(rig.emu)
        feeder.feed()
        owed = clock.tick()
        reports = chips.run(owed) if owed else {}
        if probe:
            probe.take(feeder.tick, owed, reports.get("SY0"))


def drive(ns, chips: Chips, router: rt.Router, source=None, before=None) -> dict[str, object]:  # noqa: ANN001  # pragma: no cover
    """The whole experiment; returns the result record."""
    import arm_dsp_capture as cap
    import arm_keydown as kd
    import arm_program_load as apl
    import arm_screens as sc
    from arm_bulk_tables import real_port_capture

    t0 = time.perf_counter()
    session = sc.Session(ns.image, ns.sram)
    program = ns.program.read_bytes()
    session.load_eeproms(apl.build_image(ns.eeprom.read_bytes(), program, ns.program.stem[:23]))
    kd.install_board_present(session.emu)
    real_port_capture(session.emu)
    timeline = syt.SyTimeline() if ns.sy_timeline else None
    feeder = Feeder(session.emu, router, return_observer(source), timeline)
    if before is not None:
        before.append(feeder.feed)  # a poll inside one ARM tick sees the messages sent so far
    pc, insns = boot_live(session, feeder, chips)
    result: dict[str, object] = {"boot_pc": f"{pc:#x}", "boot_insns": insns}
    result["boot"] = {s.name: snapshot(link, t0) for s, link in chips.items}
    rig = kd.Rig(session.emu, kd.Uart2().install(session.emu))
    session.emu.install_dsp_ports()
    read_log = rtn.ReadLog()
    if source:
        rtn.install(session.emu, source, read_log)
    else:
        cap.install_dsp_ack(session.emu)
    feeder.grouper.pending.clear()
    clock, phases, flash = dl.Clock(), {}, None
    prev = {s.name: snapshot(link, t0) for s, link in chips.items}
    keys = [int(k) for k in ns.keys.split(",")] if ns.keys else [ns.key]
    probe = syt.SyProbe(timeline, chips.sy_link(), source) if timeline else None
    chord = {
        i * ns.key_gap: (lambda k=k: rig.send(kd.note_on(k, VELOCITY))) for i, k in enumerate(keys)
    }
    plan = [("load", LOAD_TICKS, None, None), ("idle", IDLE_TICKS, None, None),
            ("key", ns.key_ticks, None, chord),
            ("off", OFF_TICKS, lambda: [rig.send(kd.note_off(k)) for k in keys], None)]  # fmt: skip
    starts, marks = {}, {}
    seen_prev = dict(router.seen)
    for name, ticks, action, events in plan:
        starts[name], marks[name] = chips.lead.pass_index, clock.ticks
        if action:
            action()
        run_ticks(rig, feeder, clock, chips, ticks, ns.insns_per_tick, events, probe, read_log)
        if (
            name == "idle" and ns.ns4b
        ):  # the recall flags the SAMPLE layers, the harness has no library
            import arm_piano_load as pl

            result["flagged_layers"] = cap.bind_samples(
                session.emu, pl.read_npno(ns.ns4b, cap.SAMPLE_MEMBER, 0x400000)
            )
        if name == "idle" and ns.piano_bind:
            result["piano_bind"] = bind_program_piano(session.emu, ns.piano_bind, program)
            flash = attach_smp_flash(chips, ns.piano_bind, result["piano_bind"], ns.log)
        now = {s.name: snapshot(link, t0) for s, link in chips.items}
        phases[name] = {"ticks": ticks, **{n: phase_delta(prev[n], now[n]) for n in now}}
        phases[name]["arm_messages"] = seen_delta(seen_prev, router.seen)  # every comm, D170
        seen_prev, prev = dict(router.seen), now
    if timeline:
        notes = [marks["key"] + i * ns.key_gap for i in range(len(keys))]
        result["sy_timeline"] = timeline.summary(notes, marks["off"])
        result["sy_timeline"]["note_ticks"], result["sy_timeline"]["off_tick"] = notes, marks["off"]
        sy = chips.sy_link()
        result["sy_chip"] = {"counters": sy.stats.sy, "errors": sy.stats.errors[:3]} if sy else None
    result.update(
        phases=phases,
        final=prev,
        phase_start_pass=starts,
        watch={n: w.summary() for n, w in chips.watch.items()},
        arm_messages=dict(sorted(router.seen.items())),
        skipped_out_of_window=router.skipped,
        l2_to_core2=router.l2,
    )
    if flash:
        result["smp_flash"] = smp_flash_report(flash)
    result["ticks"], result["clock_passes"] = clock.ticks, clock.passes
    if source:
        result["return_path"] = return_report(source, read_log)
    return result


def bind_program_piano(emu, backup: Path, program: bytes) -> dict:  # noqa: ANN001  # pragma: no cover
    """Bind the enabled piano layer A sound of the program (`arm_piano_bind`, OQ41)."""
    import arm_piano_bind as pb
    import arm_piano_load as pl

    layers = pb.program_piano_layers(program)
    layer = pb.active_layer(layers)
    with zipfile.ZipFile(backup) as archive:
        member = pb.resolve_member(pb.piano_members(archive.namelist()), layers[layer]["name_hash"])
    if member is None:
        return {"layer": layer, "member": None}
    npno = pl.read_npno(backup, member)
    return {"layer": layer, "member": member, **pb.bind_piano(emu, npno, layer=layer)}


def attach_smp_flash(chips, backup: Path, bound: dict, log: Path | None):  # noqa: ANN001, ANN201  # pragma: no cover
    """Serve the bound `.npno` as the flash of the live SMP chip (`arm_smp_flash`, D191)."""
    import arm_piano_load as pl
    import arm_smp_flash as sf

    link = next((lk for s, lk in chips.items if s.chip == "SMP"), None)
    if link is None or not bound.get("member"):
        return None
    npno = pl.read_npno(backup, bound["member"], 1 << 30)
    path = (log or Path(".")) / "smp_flash.npno"  # a local scratch copy, never committed
    path.parent.mkdir(parents=True, exist_ok=True)
    tap = path.with_name("smp_tap.f32") if log else None
    feed = sf.attach(link, npno, path, tap)
    return feed, link, npno, tap


def smp_flash_report(flash) -> dict:  # noqa: ANN001  # pragma: no cover
    """Descriptors seen, the chip's flash counters and the ring check."""
    import arm_smp_flash as sf

    feed, link, npno, tap = flash
    rings = sf.check_rings(link, npno, feed.strokes)
    out = {**feed.summary(), "chip": link.command("E"), "rings": rings}
    if tap is not None and tap.exists():  # the tap file is flushed per pass (D199)
        data = tap.read_bytes()
        out["tap"] = sf.tap_stats(data) | {"fundamental_cycles": sf.tap_fundamental(data)}
    return out


def replay_cmd(spec: rt.ChipSpec, exe: Path, log: Path, passes: int, tap: Path) -> list[str]:
    """`sharc-sys` command replaying `log` on the chip of `spec` (no ARM)."""
    cmd = [str(exe), spec.image, "--epdma", "--warmup", "0", "--passes", str(passes)]
    cmd += ["--msg-script", str(log)]
    if spec.preset == "pno":
        return [*cmd, "--pno", "--lane", "2:0"]
    addr, count, stride = (int(x, 0) for x in spec.window.split(":"))
    return [*cmd, "--tap", f"{addr:#x}:{count * stride}", "--tap-out", str(tap)]


def replay_levels(
    spec: rt.ChipSpec, exe: Path, log: Path, passes: int, tmp: Path
) -> dict:  # pragma: no cover
    """Level of a replay: `sharc-sys` lane (PNO preset) or the voice window tap (ORG preset)."""
    r = subprocess.run(  # noqa: S603
        replay_cmd(spec, exe, log, passes, tmp), capture_output=True, text=True, check=False
    )
    if spec.preset == "pno":
        return {"stdout_tail": r.stdout.strip().splitlines()[-6:]}
    _, count, stride = (int(x, 0) for x in spec.window.split(":"))
    peak, rms, n = rt.tap_levels(tmp.read_bytes(), count * stride, count * stride, stride)
    return {"peak": peak, "rms": rms, "samples": n, "passes": passes}


def old_replay(ns, items, result: dict) -> dict:  # noqa: ANN001  # pragma: no cover
    """D132 style: the recall chip's own log plus the key chip's words of the key and off phases."""
    by = {s.name: (s, link) for s, link in items}
    rname, kname = ns.old_replay.split(",")
    key_from = result["phase_start_pass"]["key"]
    keys = [r for r in by[kname][1].log if r[0] >= key_from]
    script = ns.log / "old_replay.script"
    script.write_text(rt.merged_script(by[rname][1].log, keys), encoding="utf-8")
    spec, link = by[rname]
    out = replay_levels(spec, ns.sys_exe, script, link.pass_index, ns.log / "old_replay.tap")
    return {"recall": rname, "key": kname, "key_words": len(keys), **out}


def specs_from_args(ns) -> list[rt.ChipSpec]:  # noqa: ANN001
    """`--chip` specs, or the D138 one-chip flags as `PNO:1=pno=IMAGE`."""
    if ns.chip:
        return [rt.parse_chip_spec(c) for c in ns.chip]
    if not ns.dsp_image:
        raise SystemExit("give --chip SPEC (repeatable) or --dsp-image IMAGE")
    return [rt.parse_chip_spec(f"PNO:1=pno={ns.dsp_image}=={','.join(ns.probe)}")]


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs the local firmware
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("image", type=Path)
    ap.add_argument("sram", type=Path)
    ap.add_argument("--eeprom", type=Path, required=True)
    ap.add_argument("--program", type=Path, required=True)
    ap.add_argument("--chip", action="append", default=[], help="chip spec (repeatable)")
    ap.add_argument("--dsp-image", type=Path, help="D138 one-chip form: PNO core 1 image")
    ap.add_argument("--probe", action="append", default=[], help="with --dsp-image")
    ap.add_argument("--live", type=Path, required=True, help="sharc-live executable")
    ap.add_argument("--sys-exe", type=Path, help="sharc-sys executable: replay the logs afterwards")
    ap.add_argument("--key", type=int, default=KEY)
    ap.add_argument("--key-ticks", type=int, default=KEY_TICKS)
    ap.add_argument("--keys", help="chord notes, comma list (default: --key)")
    ap.add_argument(
        "--poll-advance",
        type=int,
        default=0,
        help="run one SY pass per N reads of "
        "the same status word (the chip runs while the ARM polls; 0 = off)",
    )
    ap.add_argument("--key-gap", type=int, default=20, help="ticks between chord notes")
    ap.add_argument("--ns4b", type=Path, help="factory backup: bind a stand-in sample to the "
                    "flagged SAMPLE layers after the idle phase")  # fmt: skip
    ap.add_argument("--piano-bind", type=Path, metavar="BACKUP", help="bind the program's piano "
                    "sound (.npno of this backup) after the idle phase (OQ41)")  # fmt: skip
    ap.add_argument("--sy-timeline", action="store_true",
                    help="per-tick SY sample and key-down / free timing (D153)")  # fmt: skip
    ap.add_argument("--insns-per-tick", type=int, default=200_000)
    ap.add_argument("--old-replay", metavar="RECALL,KEY", help="chip names, e.g. ORG2,PNO2: the "
                    "D132 way: RECALL chip stream + the KEY chip's key and off words")  # fmt: skip
    ap.add_argument("--return-path", action="store_true",
                    help="serve the ARM SPI reads from the DSP side (D147)")  # fmt: skip
    ap.add_argument("--images", type=Path, help="dir with PNO ORG core<N>_sram.bin")
    ap.add_argument("--log", type=Path, help="directory for one msg-script per chip")
    ap.add_argument("--out", type=Path)
    ns = ap.parse_args(argv)
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    specs = specs_from_args(ns)
    router, items = rt.Router(), []
    for s in specs:
        link = dl.Link.spawn(str(ns.live), s.image, *s.flags())
        router.add(s, link)
        items.append((s, link))
    chips = Chips(items)
    try:
        before: list = []
        source = (
            return_source(items, ns.images, ns.poll_advance, before) if ns.return_path else None
        )
        result = drive(ns, chips, router, source, before)
        result["chips"] = {s.name: " ".join(s.flags()) for s in specs}
        if ns.log:
            ns.log.mkdir(parents=True, exist_ok=True)
            for s, link in items:
                with (ns.log / "skipped.txt").open("w", encoding="utf-8", newline="\n") as fh:
                    fh.writelines(f"{n} {a:#010x} {v:#010x}\n" for n, a, v in router.skipped_words)
                with (ns.log / f"{s.name}.script").open("w", encoding="utf-8", newline="\n") as fh:
                    fh.write(dl.script_text(link.log))
    finally:
        for _, link in items:
            link.close()
    if ns.sys_exe and ns.log:
        result["replay"] = {
            s.name: replay_levels(s, ns.sys_exe, ns.log / f"{s.name}.script", link.pass_index,
                                  ns.log / f"{s.name}.tap")
            for s, link in items
        }  # fmt: skip
    if ns.sys_exe and ns.log and ns.old_replay:
        result["old_replay"] = old_replay(ns, items, result)
    text = json.dumps(result, indent=1)
    if ns.out:
        ns.out.write_text(text, encoding="utf-8")
    print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
