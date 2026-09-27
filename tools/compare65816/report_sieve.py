#!/usr/bin/env python3
"""Archive passing sieve timings, complete code listings and dynamic profiles."""
import argparse
import csv
import json
from pathlib import Path
import sys

sys.dont_write_bytecode = True
from sieve import ROOT, digest, verify


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', type=Path, default=ROOT / 'target/sieve65816-comparison')
    parser.add_argument('--output', type=Path, default=ROOT / 'docs/benchmarks/65816-sieve-speed')
    args = parser.parse_args()
    src, out = args.input.resolve(), args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    load = lambda name: json.loads((src / name).read_text())
    manifest, debug, release = load('manifest.json'), load('debug.json'), load('release.json')
    verify(manifest)
    assert debug == release and manifest['crlf_checked']
    records = release['records']
    expected = {(a['compiler'], a['mode'], a['variant'], a['placement'], v['n'])
                for a in manifest['artifacts'] for v in manifest['vectors'][a['variant']]}
    key = lambda r: tuple(r[k] for k in ('compiler', 'mode', 'variant', 'placement', 'n'))
    assert len(records) == len(expected) == 126
    assert {key(r) for r in records} == expected
    assert all(not r['errors'] for r in records), 'incorrect output or ABI/access violation'
    quals = {}
    for mode in ('debug', 'release'):
        qualification = load(mode + '-qualification.json')
        assert qualification['manifest_sha256'] == digest(src / 'manifest.json')
        assert qualification['results_sha256'] == digest(src / (mode + '.json'))
        assert qualification['log_sha256'] == digest(src / (mode + '.log'))
        vm = qualification['vm_qualification']
        harness = 'tools/native65816-runtime-tests/tests/sieve_bench.rs'
        assert vm['compiler_and_fixture_inputs'][harness] == digest(ROOT / harness)
        quals[mode] = {k: vm[k] for k in ('compiler_revision', 'vm_base', 'vm_patch_sha256', 'rust', 'command')}
        quals[mode]['attestation_sha256'] = digest(src / (mode + '-qualification.json'))

    def table(name, rows):
        with (out / name).open('w', newline='') as stream:
            writer = csv.DictWriter(stream, fieldnames=list(rows[0]), lineterminator='\n')
            writer.writeheader()
            writer.writerows(rows)

    keys = ['variant', 'compiler', 'mode', 'placement', 'n', 'result', 'code_bytes', 'guard_bytes', 'rodata_bytes',
            'flags_bytes', 'cycles', 'guard_cycles', 'instructions', 'stack_reads', 'stack_writes',
            'dp_reads', 'dp_writes', 'mode_switches', 'flags_reads', 'flags_writes', 'halo_reads', 'peak_below_entry_s']
    table('measurements.csv', [{k: r[k] for k in keys} for r in records])
    rows, profiles, routines = [], [], []
    for r in records:
        if r['n'] < 8191 or r['placement'] != 'normal':
            continue
        rows.append({k: r[k] for k in ('variant', 'compiler', 'mode', 'n', 'result', 'code_bytes', 'guard_bytes',
                                       'rodata_bytes', 'flags_bytes', 'cycles', 'guard_cycles')}
                    | dict(cycles_excluding_guards=r['cycles'] - r['guard_cycles']))
        profiles.append(r)
        artifact = next(a for a in manifest['artifacts'] if all(a[k] == r[k] for k in ('compiler', 'mode', 'variant', 'placement')))
        for routine in artifact['routines']:
            sites = {pc: c for pc, c in r['instruction_sites'].items()
                     if routine['address'] <= int(pc, 16) < routine['address'] + routine['size']}
            routines.append({k: r[k] for k in ('variant', 'compiler', 'mode', 'n')}
                            | dict(routine=routine['name'], code_bytes=routine['size'],
                                   entries=sites.get(f'{routine["address"]:06X}', [0, 0])[0],
                                   exclusive_cycles=sum(c[1] for c in sites.values())))
    table('summary.csv', rows)
    table('routines.csv', routines)
    (out / 'profiles.json').write_text(json.dumps(profiles, indent=2) + '\n')
    for artifact in manifest['artifacts']:
        lines = []
        for line in (Path(artifact['directory']) / 'code.linked.lst').read_text().splitlines():
            if artifact['compiler'] == 'actionc' and line and line[0] in '0123456789ABCDEF' and not any(
                    r['address'] <= int(line[:6], 16) < r['address'] + r['size'] for r in artifact['routines']):
                continue  # Exclude the unused Action! Main routine.
            lines.append(line.rstrip())
        name = '-'.join(str(artifact[k]) for k in ('variant', 'compiler', 'mode', 'placement')) + '.lst'
        (out / name).write_text('\n'.join(lines) + '\n')
    provenance = {k: manifest[k] for k in ('compiler_revision', 'compiler_worktree_status', 'suite_revision',
                                         'suite_sieve_status', 'versions', 'tools', 'inputs', 'crlf_checked')}
    provenance.update(paired_records=len(records), executions_per_host=2 * len(records), debug_release_identical=True,
                      failed_paired_records=0, qualifications=quals,
                      archived_outputs={p.name: digest(p) for p in sorted(out.iterdir())
                                        if p.suffix in ('.csv', '.lst') or p.name == 'profiles.json'},
                      artifacts=[{k: a[k] for k in ('compiler', 'mode', 'variant', 'placement', 'commands', 'hashes')}
                                 for a in manifest['artifacts']],
                      evidence={p.name: digest(p) for p in [src / 'manifest.json', src / 'debug.json', src / 'release.json']},
                      tools_hashes={str(p.relative_to(ROOT)): digest(p) for p in [Path(__file__).resolve(),
                          ROOT / 'tools/compare65816/run_sieve.py', ROOT / 'tools/native65816-runtime-tests/tests/sieve_bench.rs']})
    (out / 'provenance.json').write_text(json.dumps(provenance, indent=2) + '\n')
    print(json.dumps(rows, indent=2))


if __name__ == '__main__':
    main()
