"""How often does each census-diff shape occur in the firmware DSP code? (numbers and addresses only)

usage: python census_freq.py IMAGE_DIR [--list]

Same sweep and tiling as `census_diff.py` / `tools/sharc_census.py` (6 core sections of the five
images), but every instruction occurrence is counted, not deduplicated. Each occurrence is labelled
with ADI's `elfdump` text and matched against the shapes the census still reports as differing.
Prints, per shape, the count per image and core (with the short-word address of the first hits when
--list is given). No instruction bytes are printed beyond the mnemonic shape.
"""
import argparse
import collections
import glob
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, '..', '..', '..', 'tools'))

SHAPES = [
    ('stkyx_lw', re.compile(r'^stky[xy]=dm\(.*\(lw\)$')),
    ('mul_dual_swap', re.compile(r'^(if [\w ]+ )?f\d+=f\d+\*f\d+, f\d+=f\d+\+f\d+, f\d+=f\d+-f\d+, r\d+<->s\d+$')),
    ('cond_lshift', re.compile(r'^if [\w ]+ r\d+=lshift r\d+ by ')),
    ('cond_swap', re.compile(r'^if [\w ]+ r\d+<->s\d+$')),
    ('mrf_write_with_move', re.compile(r'^mr[012][fb]=r\d+, (r\d+=dm|dm)\(')),
    ('cond_fpack_move', re.compile(r'^if [\w ]+ r\d+=fpack f\d+, ')),
]


def scan(image_dir):
    """-> {parcels: [(image, core, short_word_address)]} for every tiled occurrence."""
    import ns4_dsp_ldr as ldr
    import sharc_visa as v
    from sharc_census import MIN_HITS, SRAM_CODE_END, SRAM_CODE_START
    occ = collections.defaultdict(list)
    for f in sorted(glob.glob(os.path.join(image_dir, 'dsp_image_*.bin'))):
        img = open(f, 'rb').read()
        blocks, end = ldr.parse_blocks(img)
        if not blocks or end != len(img):
            continue
        image = os.path.basename(f).split('_')[3].split('.')[0]
        for ci, core in enumerate(ldr.split_cores(blocks)):
            mem = bytes(ldr.sram_image(img, core))
            for lo, hi, hits in v.sweep(mem, SRAM_CODE_START, SRAM_CODE_END):
                if hits < MIN_HITS:
                    continue
                insns, _ = v.tile(v.parcels_le(mem[lo:hi]))
                pos = lo // 2
                for _name, parcels in insns:
                    occ[tuple(parcels)].append((image, ci, pos))
                    pos += len(parcels)
    return occ


def classify_all(occ):
    import adi_dis
    order = sorted(occ)
    stream, addr = [], []
    for parcels in order:
        addr.append(len(stream))
        stream += list(parcels)
    listing = adi_dis.disassemble(stream)
    out = {}
    for parcels, a in zip(order, addr):
        entry = listing.get(a)
        if entry and entry[0] == list(parcels):
            out[parcels] = entry[1].strip().lower()
    return out


def count(occ, text):
    res = {k: collections.defaultdict(list) for k, _ in SHAPES}
    for parcels, t in text.items():
        norm = re.sub(r'\s+', ' ', t.replace(' (', '(').replace('( ', '(')).replace('(lw)', ' (lw)').strip()
        norm = norm.replace(' (lw) ', ' (lw)')
        for key, rx in SHAPES:
            if rx.search(norm) or rx.search(t):
                for image, core, pos in occ[parcels]:
                    res[key][(image, core)].append(pos)
    return res


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('images')
    ap.add_argument('--list', action='store_true')
    a = ap.parse_args()
    occ = scan(a.images)
    total = sum(len(v) for v in occ.values())
    print('%d instruction occurrences, %d distinct' % (total, len(occ)))
    res = count(occ, classify_all(occ))
    for key, _ in SHAPES:
        n = sum(len(v) for v in res[key].values())
        print('%s: %d' % (key, n))
        for (image, core), pos in sorted(res[key].items()):
            line = '  %s core %d: %d' % (image, core, len(pos))
            if a.list:
                line += ' @ ' + ' '.join('%X' % p for p in pos[:8])
            print(line)


if __name__ == '__main__':
    main()
