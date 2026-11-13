#!/usr/bin/env python3
"""Qualify and summarize core planning-layout diagnostics without changing codegen."""
import argparse
import collections
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def sha(data):
    return hashlib.sha256(data).hexdigest()


def run(args):
    return subprocess.check_output([str(a) for a in args], cwd=ROOT)


def summarize(directory):
    manifest = json.loads((directory / 'manifest.json').read_text())
    assert manifest['status'] == 'complete', manifest
    assert manifest['core_layout_census']['status'] == 'complete', manifest
    rounds = collections.Counter()
    predicates = collections.Counter()
    structural = collections.Counter()
    raw_sites = collections.Counter()
    resolutions = collections.defaultdict(collections.Counter)
    outside_sites = collections.Counter()
    examples = {}
    homes = collections.defaultdict(set)
    names = {}
    for line in (directory / 'events.jsonl').open():
        event = json.loads(line)
        data = event['data']
        if event['kind'] == 'snapshot':
            names.update({r['id']:r['name'] for r in data.get('routines',[])})
        if event['kind'] == 'core-layout-round' and data['applied_count']:
            families = sorted({p['family'] for p in data['applied']})
            structural_entries = [o for o in data['observations'] if o['observation']['predicate'] == 'ordinary-range-outside-zero-page']
            for entry in structural_entries:
                structural[', '.join(families)] += entry['evaluations']
            # Structural bounds observations have no numeric address input.
            movable = [o for o in data['observations'] if o not in structural_entries and any(i['planning_address'] for i in o['observation']['inputs'])]
            if not movable:
                continue
            # Count rounds, not individual dependencies of each applied rewrite.
            family_group = ', '.join(families)
            rounds[family_group] += 1
            for entry in movable:
                observation = entry['observation']
                for item in observation['inputs']:
                    if item['planning_address']:
                        home = item['home']
                        homes[family_group].add((home['kind'], home['routine'], home['id'], home['offset']))
                key = observation['predicate']
                if key == 'address-query':
                    for item in observation['inputs']:
                        if item['planning_address']:
                            resolutions[family_group]['unknown' if item['address'] is None else 'resolved'] += entry['evaluations']
                    raw_sites[f"{observation['file']}:{observation['line']}"] += entry['evaluations']
                else:
                    predicates[key] += entry['evaluations']
                    examples.setdefault((family_group, key), dict(attempt=event['attempt'], routine=data['routine'], generation=data['generation'], applied=data['applied'], observation=observation))
        elif event['kind'] == 'core-layout-unattributed':
            for entry in data['observations']:
                observation = entry['observation']
                if observation['predicate'] != 'ordinary-range-outside-zero-page' and any(i['planning_address'] for i in observation['inputs']):
                    outside_sites[f"{observation['file']}:{observation['line']}"] += entry['evaluations']
    return dict(census=manifest['core_layout_census'], work=manifest['work'],
                bytes=manifest['output_sizes']['load_file_bytes'],
                locally_applied_rounds_with_planning_queries=dict(rounds),
                planning_query_resolution_by_applied_family_group={family:dict(counts) for family,counts in resolutions.items()},
                predicate_evaluations_in_those_rounds=dict(predicates),
                structural_evaluations_by_applied_family_group=dict(structural),
                planning_homes_by_applied_family_group={family:[dict(kind=h[0],routine=h[1],id=h[2],offset=h[3]) for h in sorted(values,key=lambda h:(h[0],-1 if h[1] is None else h[1],h[2],h[3]))] for family,values in homes.items()},
                raw_query_evaluations_by_site=dict(raw_sites),
                unattributed_query_evaluations_by_site=dict(outside_sites),
                examples=[dict(example, routine_name=names.get(example['routine'])) for example in examples.values()],
                interpretation='Local applied discovery rounds, including speculative materializations; query sets may include rejected candidates. No per-rewrite necessity or final survival inferred.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', type=Path, default=ROOT / 'build/checked-arena/shared-target/debug/actionc-emit')
    parser.add_argument('--api-rlib', type=Path)
    parser.add_argument('--output', type=Path, default=ROOT / 'build/mir6502-layout-census/qualification')
    args = parser.parse_args()
    compiler = args.compiler.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix='run-', dir=args.output)).resolve()
    frozen = json.loads((ROOT / 'docs/benchmarks/6502-late-spills/qualification.json').read_text())
    catalog = json.loads((ROOT / 'docs/benchmarks/6502-phase-tracing/baseline.json').read_text())
    expected = {c['id']: c for c in frozen['cli_cases']}
    report = dict(baseline_revision='2c6b8edf', implementation_revision=run(['git','rev-parse','HEAD']).decode().strip(), compiler_sha256=sha(compiler.read_bytes()), cli_cases=[])
    for case in catalog['cases']:
        assert sha((ROOT / case['source']).read_bytes().replace(b'\r\n', b'\n')) == case['source_sha256']
        command = [compiler, '--profile', case['profile'], '--backend', 'mir6502', '--runtime', case['runtime'], '--origin', case['origin']]
        artifacts = {}
        for suffix, mode in [('xex','--emit-load'),('asm','--emit-listing'),('map','--emit-map')]:
            data = run(command + [mode,case['source']])
            assert sha(data) == expected[case['id']]['artifacts'][suffix]['sha256'], (case['id'],suffix)
            artifacts[suffix] = dict(sha256=sha(data), bytes=len(data))
        report['cli_cases'].append(dict(id=case['id'],artifacts=artifacts))
        print(case['id'] + ': frozen artifacts unchanged', flush=True)
    rlib = args.api_rlib or max((compiler.parent / 'deps').glob('libactionc-*.rlib'), key=lambda p:p.stat().st_mtime)
    report['api_rlib_sha256'] = sha(rlib.read_bytes())
    probe = work / ('probe.exe' if sys.platform == 'win32' else 'probe')
    run(['rustc','--edition=2024',ROOT / 'tools/compare6502/core_layout_census.rs','--extern',f'actionc={rlib}','-L',f'dependency={rlib.parent}','-o',probe])
    output = run([probe,work / 'api'])
    (work / 'api.jsonl').write_bytes(output)
    rows = [json.loads(line) for line in output.decode().splitlines()]
    assert len(rows) == 16
    report['api'] = [dict(id=row['id'],**summarize(work / 'api' / row['id'])) for row in rows]
    report['source_sha256_lf'] = {p:sha((ROOT / p).read_bytes().replace(b'\r\n',b'\n')) for p in [
        'samples/tn/modern/TN.ACT','fixtures/runtime/word_load_sharing.act',
        'fixtures/runtime/argument_staging.act','fixtures/runtime/pointer_invariant_loop.act',
        'fixtures/runtime/core_layout_census.act']}
    report['cli_artifact_comparisons'] = 54
    report['full_api_comparisons'] = 16
    (work / 'qualification.json').write_text(json.dumps(report,indent=2)+'\n')
    print(work / 'qualification.json')


if __name__ == '__main__':
    main()
