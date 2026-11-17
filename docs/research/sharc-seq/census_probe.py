"""Runs hand-picked instructions on the simulator and the model with random core states and lists the differences.

usage: python census_probe.py cens.dxe census_fixture.txt --work DIR --parcels HEX [HEX ...] [--states K] [--modes M ...]
       python census_probe.py ... --parcels-file FILE      (one instruction per line, `#` comments)

Same cases file format, simulator runner and model runner as `census_diff.py` (they share the
probe `gen_census_probe.py`), but the instructions come from the command line instead of the
firmware sweep. Use it to measure how the simulator treats one instruction family (shift counts,
conditions, swaps ...) before fixing the model. `--keep NAME` also writes `NAME_cases.txt` and
`NAME_measured.txt` (the compact format of `census_regress.py`) for a committed regression fixture.
"""
import argparse
import os
import random
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import census_diff as cd  # noqa: E402
import census_regress as cr  # noqa: E402


def parse_parcels(hexstr):
    return [int(hexstr[i:i + 4], 16) for i in range(0, len(hexstr), 4)]


def build(parcel_list, states, modes, scr_addr, seed, path):
    rnd = random.Random(seed)
    scr = [rnd.getrandbits(32) for _ in range(cd.SCR_WORDS)]
    cases = []
    with open(path, 'w') as out:
        out.write('scr ' + ' '.join('%08X' % w for w in scr) + '\n')
        for hx in parcel_list:
            for k in range(states):
                st = cd.make_state(rnd, scr_addr, modes[k % len(modes)])
                line = hx + ' ' + ' '.join('%08X' % w for w in st)
                out.write(line + '\n')
                cases.append((hx, k, line))
    return scr, cases


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('dxe')
    ap.add_argument('fixture')
    ap.add_argument('--work', required=True)
    ap.add_argument('--parcels', nargs='*', default=[])
    ap.add_argument('--parcels-file')
    ap.add_argument('--states', type=int, default=6)
    ap.add_argument('--seed', type=int, default=7)
    ap.add_argument('--modes', nargs='*', type=lambda s: int(s, 16), default=cd.MODES)
    ap.add_argument('--keep')
    a = ap.parse_args()
    plist = list(a.parcels)
    if a.parcels_file:
        plist += [l.split('#')[0].strip() for l in open(a.parcels_file) if l.split('#')[0].strip()]
    os.makedirs(a.work, exist_ok=True)
    syms = {l.split()[1]: int(l.split()[2], 16) for l in open(a.fixture) if l.startswith('sym ')}
    cpath, simo, modo = (os.path.join(a.work, n) for n in ('p_cases.txt', 'p_sim.txt', 'p_model.txt'))
    scr, cases = build(plist, a.states, a.modes, syms['_scr'], a.seed, cpath)
    subprocess.check_call([sys.executable, os.path.join(HERE, 'sim_batch.py'), a.dxe, cpath, '--output', simo],
                          stdout=subprocess.DEVNULL)
    with open(modo, 'w') as f:
        subprocess.check_call(['cargo', 'run', '-q', '-p', 'nord-sharc', '--bin', 'sharc_census_run', '--',
                               a.fixture, cpath], stdout=f)
    names = cd.layout()
    sim, mod = open(simo).read().split('\n'), open(modo).read().split('\n')
    bad = 0
    for i, (hx, k, _line) in enumerate(cases):
        if sim[i] == mod[i]:
            continue
        bad += 1
        x, y = sim[i].split(), mod[i].split()
        if x[:1] == ['HALT'] or y[:1] == ['FAULT']:
            print('%s state %d: sim %s model %s' % (hx, k, sim[i][:30], mod[i][:30]))
            continue
        diff = ['%s sim=%s model=%s' % (names[j] if j < cd.gen.N_STATE else 'scr%d' % (j - cd.gen.N_STATE),
                                       x[j], y[j]) for j in range(min(len(x), len(y))) if x[j] != y[j]][:8]
        print('%s state %d: %s' % (hx, k, '; '.join(diff)))
    print('%d of %d cases differ' % (bad, len(cases)))
    if a.keep:
        with open(a.keep + '_cases.txt', 'w') as f:
            f.write('scr ' + ' '.join('%08X' % w for w in scr) + '\n')
            f.writelines(c[2] + '\n' for c in cases)
        with open(a.keep + '_measured.txt', 'w') as f:
            f.writelines(cr.compact(s, scr) + '\n' for s in sim[:len(cases)])


if __name__ == '__main__':
    main()
