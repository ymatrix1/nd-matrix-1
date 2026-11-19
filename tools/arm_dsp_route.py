"""Several live chips on one ARM stream (D144): routing, chip specs, watched words, replay logs.

Pure parts (no firmware, no unicorn), tested in `tools/tests/test_arm_dsp_route.py`. The driver is
`tools/arm_dsp_live.py`; the one-chip transport is `tools/arm_dsp_link.py` (D138).

`Router` sends every ARM message to the chip that owns `(comm, core, path)`: the comm index is the
download order FX 0, PNO 1, ORG 2, SY 3, SMP 4 (ASSUMED, `arm-param-messages.md` s3). All chips
share one `arm_dsp_link.Clock`, so after every 24 kHz ARM tick every chip runs the same number of
passes.
"""

from __future__ import annotations

from array import array
from collections.abc import Iterable
from dataclasses import dataclass, field

CHIPS = {"FX": 0, "PNO": 1, "ORG": 2, "SY": 3, "SMP": 4}
PRESETS = ("pno", "org", "sy")  # `sharc-live` flags (`runner/live_presets.rs`); `sy` is D153
DM_WORDS = 0x100000  # data word space of one core window (`0x28000000 + 4 w`, w < 2^20)
# VERIFIED (`synth-stage-models-12.md` s1): the ARM writes the wavetables of the SUPER 2..4 and
# WAVETABLE stages to the DSP L2 byte window below the core windows; the DSP reads them as data
# words `byte >> 2` (`0x200C0000` is `DM(0x08030000)`). Both cores of a chip see the same L2.
L2_LOW, SYS_BASE = 0x20000000, 0x28000000


@dataclass
class ChipSpec:
    """One live chip: `CHIP:CORE=PRESET=IMAGE[=WINDOW[=PROBES[=WATCH]]]` on the command line."""

    chip: str
    core: int
    preset: str
    image: str
    window: str = ""  # ADDR:COUNT:STRIDE of the level statistic (empty: output lane 2:0)
    probes: list[str] = field(default_factory=list)
    watch: list[int] = field(default_factory=list)

    @property
    def name(self) -> str:
        return f"{self.chip}{self.core}"

    @property
    def comm(self) -> int:
        return CHIPS[self.chip]

    @property
    def path(self) -> int:
        """ARM message path this chip receives: SY takes path 1 (DM word addresses), the rest 2."""
        return 1 if self.preset == "sy" else 2

    def flags(self) -> list[str]:
        """`sharc-live` flags of this chip."""
        if self.preset == "sy":  # interrupt driven pass, no epdma, no level window (D153)
            return ["--sy", *(["--epchain"] if self.window == "epchain" else [])]
        out = [f"--{self.preset}", "--epdma"]
        if self.window:
            out += ["--window", self.window]
        return out + [x for p in self.probes for x in ("--probe", p)]


def _split(text: str) -> list[str]:
    return [x for x in text.split(",") if x]


def parse_chip_spec(text: str) -> ChipSpec:
    """Parse `PNO:2=org=IMAGE=0xC1F83:4:2=0x1256AD,0x121E2B=0xC1174,0xC1180` (tail optional)."""
    parts = text.split("=")
    if not 3 <= len(parts) <= 6:
        raise ValueError(f"bad chip spec {text!r}")
    head, preset, image, *rest = parts
    chip, _, core = head.partition(":")
    if chip not in CHIPS or not core.isdigit() or int(core) not in (0, 1, 2):
        raise ValueError(f"bad chip {head!r} (want FX|PNO|ORG:1|2 or SY:0)")
    if preset not in PRESETS:
        raise ValueError(f"bad preset {preset!r} (want one of {PRESETS})")
    if (core == "0") != (chip == "SY") or (preset == "sy") != (chip == "SY"):
        raise ValueError(f"bad chip {head!r} for preset {preset!r} (SY:0 takes the sy preset)")
    rest += [""] * (3 - len(rest))
    return ChipSpec(
        chip,
        int(core),
        preset,
        image,
        rest[0],
        _split(rest[1]),
        [int(x, 0) for x in _split(rest[2])],
    )


class Router:
    """Owner table `(comm, core, path)` -> link; counts every message by name, core and path."""

    def __init__(self) -> None:
        self.owner: dict[tuple[int, int, int], object] = {}
        self.seen: dict[str, int] = {}
        self.skipped: dict[str, int] = {}
        self.l2: dict[str, int] = {}  # L2 stores handed to a core 2 chip, by chip name

    def add(self, spec: ChipSpec, link: object, path: int | None = None) -> None:
        """`link` receives the messages of `spec`: path 2 (system addresses), path 1 for SY."""
        self.owner[(spec.comm, spec.core, spec.path if path is None else path)] = link

    def dispatch(self, path: int, comm: int, core: int, dm_word: int, value: int) -> str:
        """Route one message: `sent`, `skipped` (outside the core window) or `counted` (no chip)."""
        name = f"{tuple(CHIPS)[comm]}{core}p{path}"
        self.seen[name] = self.seen.get(name, 0) + 1
        if path == 2 and core == 1 and dm_word < 0:
            self.forward_l2(comm, path, SYS_BASE + 4 * dm_word, value)
        link = self.owner.get((comm, core, path))
        if link is None:
            return "counted"
        if not 0 <= dm_word < DM_WORDS:
            self.skipped[name] = self.skipped.get(name, 0) + 1
            return "skipped"
        link.send(dm_word, value)  # type: ignore[attr-defined]
        return "sent"

    def forward_l2(self, comm: int, path: int, byte: int, value: int) -> bool:
        """Hand a path 2 store into the L2 window (`arm_emu.Message` files it under core 1 with a
        negative data word) to the live core 2 chip of the same comm as `DM(byte >> 2)`, where the
        synth stages read the wavetables (OQ40). Logged, not counted as an ARM message."""
        peer = self.owner.get((comm, 2, path))
        if peer is None or not L2_LOW <= byte < SYS_BASE:
            return False
        peer.poke(byte >> 2, value)  # type: ignore[attr-defined]
        name = f"{tuple(CHIPS)[comm]}2"
        self.l2[name] = self.l2.get(name, 0) + 1
        return True


@dataclass
class WatchStats:
    """Time series summary of watched DM words (one sample per ARM tick batch)."""

    first_change: dict[int, int] = field(default_factory=dict)  # word -> pass of first change
    first_value: dict[int, int] = field(default_factory=dict)
    last: dict[int, int] = field(default_factory=dict)
    hi: dict[int, float] = field(default_factory=dict)
    lo: dict[int, float] = field(default_factory=dict)

    def add(self, pass_index: int, words: dict[int, int]) -> None:
        """Fold one sample `{address: bits}` taken after `pass_index` passes."""
        for a, bits in words.items():
            f = array("f", bits.to_bytes(4, "little"))[0]
            self.first_value.setdefault(a, bits)
            if bits != self.first_value[a]:
                self.first_change.setdefault(a, pass_index)
            self.last[a] = bits
            self.hi[a] = max(self.hi.get(a, f), f)
            self.lo[a] = min(self.lo.get(a, f), f)

    def summary(self) -> dict[str, object]:
        """JSON friendly: first change pass, final value, float range."""
        return {
            f"{a:#x}": {
                "first_change_pass": self.first_change.get(a),
                "final": f"{self.last[a]:#010x}",
                "float_min": self.lo[a],
                "float_max": self.hi[a],
            }
            for a in sorted(self.last)
        }


def merged_script(*logs: Iterable[tuple[int, int, int]]) -> str:
    """Several `(pass, word, value)` logs as one `sharc-sys --msg-script`, sorted by pass."""
    rows = sorted((r for log in logs for r in log), key=lambda r: r[0])
    return "".join(f"{p} {w:#x} {v:#010x}\n" for p, w, v in rows)


def tap_levels(
    raw: bytes, width: int, lane_words: int, stride: int = 2
) -> tuple[float, float, int]:
    """`(peak, rms, samples)` of the first `lane_words` of every `width`-word tap record.

    `raw` is a `sharc-sys --tap-out` file (little-endian words, pass-major); every `stride`-th
    word of the window is a sample (the PEx lane, `synth-e2e-oracle-vs-native.md` s2).
    """
    words = array("f")
    words.frombytes(raw[: len(raw) // 4 * 4])
    peak = sumsq = 0.0
    n = 0
    for base in range(0, len(words) - width + 1, width):
        for x in words[base : base + lane_words : stride]:
            if x == x and abs(x) != float("inf"):
                peak, sumsq, n = max(peak, abs(x)), sumsq + x * x, n + 1
    return peak, (sumsq / n) ** 0.5 if n else 0.0, n
