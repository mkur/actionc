#!/usr/bin/env python3
"""Archive the post-slice-8 checkpoint; Exec is a build comparison only."""
import argparse
import csv
import json
from pathlib import Path
import re
import sys

sys.dont_write_bytecode = True
from build import ROOT, digest, image_guard_ranges


def read(path):
    return json.loads(Path(path).read_text())


def table(path, rows):
    with path.open('w', newline='') as output:
        writer = csv.DictWriter(output, fieldnames=list(rows[0]), lineterminator='\n')
        writer.writeheader()
        writer.writerows(rows)


def verify_artifact(artifact):
    for name, expected in artifact['hashes'].items():
        assert digest(Path(artifact['directory']) / name) == expected, name


def routine_rows(group, mode, before, after):
    def index(image):
        result = {}
        for routine in image['routines']:
            segment = next(s for s in image['segments'] if s['address'] == routine['address'])
            guards = image_guard_ranges(image, segment)
            result[routine['name']] = dict(code_bytes=routine['size'],
                guard_bytes=sum(end-start for start, end in guards),
                fixed_frame=routine['fixed_frame'], local_stack_peak=routine['local_stack_peak'],
                spill_bytes=routine['spill_bytes'],
                dp_home_bytes=len({t['home']['offset']+b for t in routine['temporaries']
                    if t['home']['kind'] == 'direct_page' for b in range(t['size'])}))
        return result
    old, new = index(before), index(after)
    assert old.keys() == new.keys()
    return [dict(group=group, mode=mode, routine=name) |
            {f'{key}_{suffix}': value for suffix, values in [('before', old[name]), ('after', new[name])]
             for key, value in values.items()} for name in new]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', type=Path, required=True)
    parser.add_argument('--baseline', type=Path, required=True)
    parser.add_argument('--exec-build-root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    src, old, out = args.input, args.baseline, args.output
    out.mkdir(parents=True, exist_ok=True)
    files, routines, totals = {}, [], []
    def record(path):
        path = Path(path)
        files[str(path.resolve())] = digest(path)
        return read(path)
    manifests = {}
    for group in ['corpus', 'dijkstra']:
        before = record(old/group/'manifest.json')
        after = record(src/group/'manifest.json')
        assert after['crlf_checked']
        for path, expected in after['inputs'].items():
            assert digest(path) == expected, path
        for tool in after['tools'].values():
            assert digest(tool['path']) == tool['sha256']
        manifests[group] = after
        for artifact in after['artifacts']:
            verify_artifact(artifact)
            key = lambda a: (a['compiler'], a['mode'], a.get('case'))
            baseline = next(a for a in before['artifacts'] if key(a) == key(artifact))
            verify_artifact(baseline)
            if artifact['compiler'] != 'actionc':
                # Foreign compiler output is not re-executed or a correctness oracle.
                continue
            name = group if group == 'dijkstra' else 'corpus/'+artifact['case']
            rows = routine_rows(name, artifact['mode'], record(baseline['image']), record(artifact['image']))
            routines.extend(rows)
            totals.append(dict(group=name, mode=artifact['mode'],
                code_bytes_before=baseline['code_bytes'], code_bytes_after=artifact['code_bytes'],
                guard_bytes_before=sum(r['guard_bytes_before'] for r in rows),
                guard_bytes_after=sum(r['guard_bytes_after'] for r in rows)))

    debug, release = [record(src/f'corpus-{host}.json') for host in ['debug', 'release']]
    assert debug == release
    corpus = release['measurements']
    manifest = record(src/'corpus/action-only-manifest.json')
    assert release['manifest'] == manifest
    expected = {(a['mode'], a['case'], i) for a in manifest['artifacts']
                for c in manifest['cases'] if a['case'] == c['id'] for i in range(len(c['vectors']))}
    assert len(corpus) == len(expected) == 132
    assert {(r['mode'], r['case'], r['vector']) for r in corpus} == expected
    assert all(r['correct'] and not r['errors'] and r['compiler'] == 'actionc' for r in corpus)
    metrics = ['cycles', 'instructions', 'stack_reads', 'stack_writes', 'dp_reads', 'dp_writes',
               'metadata_reads', 'peak_below_entry_s', 'stack_check_cycles']
    table(out/'corpus-results.csv', [{k: r[k] for k in ['mode', 'case', 'vector', 'code_bytes', *metrics]} for r in corpus])

    dijkstra = record(src/'dijkstra-release.json')
    assert not dijkstra['filter']
    results = dijkstra['results']
    labels = [line.split()[0] for line in Path(manifests['dijkstra']['vectors']).read_text().splitlines()
              if line and not line.startswith('#')]
    assert len(results) == 66 and {(r['mode'], r['case']) for r in results} == {
        (mode, label) for mode in ['raw', 'optimized'] for label in labels}
    assert all(not r['errors'] and r['interrupt_masks'] == [0, 4] and r['compiler'] == 'actionc' for r in results)
    table(out/'dijkstra-results.csv', [{k: r[k] for k in ['mode', 'case', *metrics, 'mode_switches']} for r in results])
    historical = record(ROOT/'docs/benchmarks/65816-constant-shifts/dijkstra-execution.json')
    equality = record(ROOT/'docs/benchmarks/65816-wide-returns/dijkstra-equality.json')
    record(ROOT/'docs/benchmarks/65816-captured-byte-returns/dijkstra-equality.json')
    for artifact in equality['artifacts']:
        if artifact['compiler'] == 'actionc':
            assert digest(old/'dijkstra/actionc'/artifact['mode']/'image.json') == artifact['after_file_hashes']['image.json']
    deltas = []
    for row in historical['records']:
        if row['compiler'] != 'actionc':
            continue
        current = next(r for r in results if r['mode'] == row['mode'] and r['case'] == row['case'])
        deltas.append(dict(mode=row['mode'], case=row['case']) |
            {key: current[key]-value for name, value in row.items()
             if name.endswith('_after') for key in [name.removesuffix('_after')]})
    table(out/'dijkstra-original-0-50-delta.csv', deltas)

    exec_facts = []
    for mode, suffix in [('raw', 'raw'), ('optimized', 'opt')]:
        directories = [args.exec_build_root/f'shell-{label}-{suffix}' for label in ['wide-returns', 'pointer-micro8']]
        builds = [record(d/'build.json') for d in directories]
        same = ['compiler_contract', 'abi_sha256', 'abi_assembly_sha256', 'source_sha256',
                'platform_inputs', 'optimize', 'stack_checks', 'tasks', 'task_inputs',
                'console_inputs', 'banked_inputs', 'memory_sha256', 'dos_mounts', 'exec_build']
        assert all(builds[0][key] == builds[1][key] for key in same)
        assert builds[1]['stack_checks']
        for path in directories[0].rglob('*.act'):
            other = directories[1]/path.relative_to(directories[0])
            normalize = lambda p, base: p.read_text().replace('\r\n', '\n').replace(str(base), '<BUILD>')
            assert normalize(path, directories[0]) == normalize(other, directories[1]), path
        memories = [record(d/'memory.json') for d in directories]
        assert memories[0]['bank_zero_budget'] == memories[1]['bank_zero_budget']
        images = [record(d/'program.a816.json') for d in directories]
        rows = routine_rows('exec', mode, *images)
        routines.extend(rows)
        totals.append(dict(group='exec', mode=mode,
            code_bytes_before=sum(len(s['bytes']) for s in images[0]['segments'] if s['executable']),
            code_bytes_after=sum(len(s['bytes']) for s in images[1]['segments'] if s['executable']),
            guard_bytes_before=sum(r['guard_bytes_before'] for r in rows),
            guard_bytes_after=sum(r['guard_bytes_after'] for r in rows)))
        for directory, build in zip(directories, builds):
            assert digest(directory/'program.a816.json') == build['image_sha256']
            assert digest(directory/'program.xex') == build['xex_sha256']
        exec_facts.append(dict(mode=mode, matching_build_fields=same,
            bank_zero_budget=memories[1]['bank_zero_budget'], bank_zero_budget_delta=0,
            xex_bytes_before=(directories[0]/'program.xex').stat().st_size,
            xex_bytes_after=(directories[1]/'program.xex').stat().st_size,
            builds=[{k: b[k] for k in ['revision', 'binary_sha256', 'image_sha256', 'xex_sha256',
                'exec_build', 'dos_mounts', 'source_sha256', 'abi_sha256']} for b in builds]))
    table(out/'sizes.csv', totals)
    table(out/'routine-sizes.csv', routines)
    qualifications = []
    for log in [src.parent/f'checkpoint-{host}.log' for host in ['debug', 'release']] + [
            src/f'corpus-{host}.log' for host in ['debug', 'release']] + [src/'dijkstra-release.log']:
        content = log.read_text()
        path = Path(re.findall(r'Qualification manifest: (.+)', content)[-1])
        q = record(path)
        for name, expected_hash in q['compiler_and_fixture_inputs'].items():
            assert digest(ROOT/name) == expected_hash, name
        counts = [tuple(map(int, m)) for m in re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored', content)]
        assert counts and all(failed == 0 for _, failed, _ in counts)
        qualifications.append(dict(log=str(log), log_sha256=digest(log), manifest=str(path),
            manifest_sha256=digest(path), command=q['command'], vm_base=q['vm_base'],
            vm_patch_sha256=q['vm_patch_sha256'], passed=sum(c[0] for c in counts),
            ignored=sum(c[2] for c in counts)))
    provenance = dict(scope='Post-slice-8 compiler checkpoint. Exec build comparison only; no hosted Exec qualification.',
        compiler_revision='843b3b0b', baseline_revision='87feebf1', crlf_checked=True,
        corpus_paired_mask_records_per_host=132, corpus_debug_release_identical=True,
        dijkstra_paired_mask_records_release=66, qualifications=qualifications,
        exec_builds=exec_facts, input_hashes=files, report_tool_sha256=digest(__file__),
        report_hashes={p.name: digest(p) for p in out.glob('*.csv')})
    (out/'provenance.json').write_text(json.dumps(provenance, indent=2).replace(str(ROOT), '<repo>')
        .replace(str(Path.home()), '<home>')+'\n')
    for row in totals:
        if row['code_bytes_before'] != row['code_bytes_after']:
            print(row)
    print('Dijkstra deltas:', deltas)
    print('Qualification:', [(q['log'], q['passed'], q['ignored']) for q in qualifications])


if __name__ == '__main__':
    main()
