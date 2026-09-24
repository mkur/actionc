#!/usr/bin/env python3
"""Archive an Action-only pointer slice against the preceding list measurements."""
import argparse
import csv
import hashlib
import json
from pathlib import Path


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def table(path, rows):
    with path.open('w', newline='') as f:
        writer = csv.DictWriter(f, fieldnames=list(rows[0]), lineterminator='\n')
        writer.writeheader()
        writer.writerows(rows)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--input', required=True, type=Path)
    p.add_argument('--baseline', required=True, type=Path)
    p.add_argument('--output', required=True, type=Path)
    args = p.parse_args()
    src, old, out = args.input, args.baseline, args.output
    load = lambda path: json.loads(path.read_text())
    manifest = load(src/'manifest.json')
    debug, release = load(src/'debug.json'), load(src/'release.json')
    assert debug == release and debug['manifest'] == manifest
    assert manifest['crlf_checked']
    for path, sha in manifest['inputs'].items():
        assert digest(path) == sha, path
    for tool in manifest['tools'].values():
        assert digest(tool['path']) == tool['sha256'], tool
    for artifact in manifest['artifacts']:
        assert artifact['compiler'] == 'actionc'
        for path, sha in artifact['hashes'].items():
            assert digest(Path(artifact['directory'])/path) == sha, path
    records = debug['measurements']
    key = lambda r: (r['mode'], r['case'], int(r['vector']))
    expected = {(a['mode'], a['case'], i) for a in manifest['artifacts']
                for c in manifest['cases'] if c['id'] == a['case']
                for i in range(len(c['vectors']))}
    assert len(records) == len(expected) and {key(r) for r in records} == expected
    assert all(r['correct'] and r['compiler'] == 'actionc' for r in records)
    with (old/'measurements.csv').open(newline='') as f:
        baseline = {key(r): r for r in csv.DictReader(f) if r['compiler'] == 'actionc'}
    assert baseline.keys() == expected
    metrics = ['code_bytes', 'cycles', 'stack_check_cycles', 'instructions',
               'dp_reads', 'dp_writes', 'stack_reads', 'stack_writes', 'metadata_reads',
               'input_padding_reads', 'peak_below_entry_s', 'static_stack_check_bytes']
    out.mkdir(parents=True, exist_ok=True)
    table(out/'measurements.csv', [{k: r[k] for k in
          ['compiler', 'mode', 'case', 'vector', 'correct', *metrics]} for r in records])
    table(out/'delta.csv', [dict(zip(['mode', 'case', 'vector'], key(r))) |
          {k: int(r[k]) - int(baseline[key(r)][k]) for k in metrics} for r in records])
    sizes = load(src/'sizes.json')
    table(out/'sizes.csv', [{k: r[k] for k in ['mode', 'routine', 'action_bytes',
          'action_guard_bytes', 'action_body_bytes', 'action_frame']}
          for r in sizes if r['routine'] != 'Shared helpers'])
    for mode in ['raw', 'optimized']:
        text = (src/'actionc'/mode/'code.linked.lst').read_text()
        (out/f'actionc-{mode}.lst').write_text('\n'.join(s.rstrip() for s in text.splitlines())+'\n')
    provenance = dict(compiler_revision=manifest['compiler_revision'],
        compiler_worktree_status=manifest['compiler_worktree_status'],
        tools=manifest['tools'], inputs=manifest['inputs'], crlf_checked=True,
        debug_release_identical=True, paired_mask_records=len(records), executions_per_host=2*len(records),
        comparison_exit='Both Action-only qualification runs pass; foreign compiler results remain in the original analysis.',
        baseline={str(old/n): digest(old/n) for n in ['measurements.csv', 'sizes.csv']},
        results={str(src/n): digest(src/n) for n in ['manifest.json', 'debug.json', 'release.json', 'debug.log', 'release.log']},
        artifacts=[dict(mode=a['mode'], hashes=a['hashes']) for a in manifest['artifacts'] if a['case']=='NewList'],
        report_tool_sha256=digest(__file__),
        report_artifacts={p.name: digest(p) for p in out.iterdir() if p.is_file() and p.name!='provenance.json'})
    (out/'provenance.json').write_text(json.dumps(provenance, indent=2)+'\n')
    for mode in ['raw', 'optimized']:
        rows = [r for r in records if r['mode'] == mode]
        print(mode, 'bytes', sum(r['action_bytes'] for r in sizes if r['mode'] == mode),
              'cycle delta range', min(r['cycles']-int(baseline[key(r)]['cycles']) for r in rows),
              max(r['cycles']-int(baseline[key(r)]['cycles']) for r in rows))


if __name__ == '__main__':
    main()
