#!/usr/bin/env python3
"""Qualify pointer setup against original and candidate-attributed baselines."""
import argparse
import collections
import json
from pathlib import Path
import sys
import tempfile

from core_layout_census import ROOT, run, sha


def key(data):
    return data['family'], data['block'], data['start']


def predicate_label(observation):
    predicate = observation['predicate']
    if predicate == 'ordinary-range-outside-zero-page':
        predicate += ':true' if observation['result'] else ':false'
    return predicate


def check_local_pointer_proof(before, after):
    for outcome in ['overlap-rejected', 'shape-only']:
        label = 'pointer-temp-deref/' + outcome
        old = before['local_gate_evaluations'].get(label, {})
        new = after['local_gate_evaluations'].get(label, {})
        if old:
            assert old['address-query'] > 0
            assert new == {'ordinary-range-outside-zero-page:true': old['address-query']}, (label, old, new)


def summarize(directory):
    manifest = json.loads((directory / 'manifest.json').read_text())
    assert manifest['status'] == 'complete'
    assert manifest['core_layout_census']['status'] == 'complete'
    outcomes = collections.defaultdict(collections.Counter)
    local = collections.defaultdict(collections.Counter)
    shared = collections.Counter()
    outside = collections.Counter()
    names = {}
    examples = {}
    for line in (directory / 'events.jsonl').open():
        event = json.loads(line)
        data = event['data']
        if event['kind'] == 'snapshot':
            names.update({r['id']: r['name'] for r in data.get('routines', [])})
        if event['kind'] == 'core-layout-round':
            assert data['omitted'] == 0
            candidates = {key(c['key']): c for c in data['candidates']}
            for c in candidates.values():
                outcomes[c['key']['family']][c['outcome']] += 1
            for entry in data['observations']:
                observation = entry['observation']
                predicate = predicate_label(observation)
                owner = observation.get('candidate')
                if owner is None:
                    shared[predicate] += entry['evaluations']
                else:
                    c = candidates[key(owner)]
                    label = owner['family'] + '/' + c['outcome']
                    local[label][predicate] += entry['evaluations']
                    examples.setdefault((label, predicate), dict(routine=data['routine'], attempt=event['attempt'], generation=data['generation'], candidate=c, observation=observation))
        elif event['kind'] == 'core-layout-unattributed':
            for entry in data['observations']:
                outside[predicate_label(entry['observation'])] += entry['evaluations']
    return dict(bytes=manifest['output_sizes']['load_file_bytes'], work=manifest['work'], status='complete',
                candidate_outcomes={f: dict(c) for f,c in sorted(outcomes.items())},
                local_gate_evaluations={f: dict(c) for f,c in sorted(local.items())},
                unscoped_round_evaluations=dict(shared), outside_driver_evaluations=dict(outside),
                examples=[dict(e, routine_name=names.get(e['routine'])) for e in examples.values()])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--original-api', required=True, type=Path)
    parser.add_argument('--attributed-api', required=True, type=Path)
    parser.add_argument('--compiler', type=Path, default=ROOT/'build/checked-arena/shared-target/debug/actionc-emit')
    parser.add_argument('--api-rlib', type=Path)
    parser.add_argument('--output', type=Path, default=ROOT/'build/mir6502-pointer-setup-proof/qualification')
    args = parser.parse_args()
    compiler = args.compiler.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix='run-', dir=args.output)).resolve()
    report = dict(original_revision='2b0530ac', attributed_revision='31908c9f', implementation_revision=run(['git','rev-parse','HEAD']).decode().strip(), compiler_sha256=sha(compiler.read_bytes()), cli_cases=[], api=[])
    catalog = json.loads((ROOT/'docs/benchmarks/6502-phase-tracing/baseline.json').read_text())
    frozen = json.loads((ROOT/'docs/benchmarks/6502-word-result-proof/qualification.json').read_text())
    expected = {c['id']: c for c in frozen['cli_cases']}
    for case in catalog['cases']:
        assert sha((ROOT/case['source']).read_bytes().replace(b'\r\n',b'\n')) == case['source_sha256']
        command = [compiler,'--profile',case['profile'],'--backend','mir6502','--runtime',case['runtime'],'--origin',case['origin']]
        artifacts = {}
        for suffix,mode in [('xex','--emit-load'),('asm','--emit-listing'),('map','--emit-map')]:
            data = run(command + [mode,case['source']])
            assert sha(data) == expected[case['id']]['artifacts'][suffix]['sha256'], (case['id'],suffix)
            artifacts[suffix] = dict(sha256=sha(data), bytes=len(data))
        report['cli_cases'].append(dict(id=case['id'], artifacts=artifacts))
        print(case['id'] + ': frozen artifacts unchanged', flush=True)
    rlib = args.api_rlib or max((compiler.parent/'deps').glob('libactionc-*.rlib'),key=lambda p:p.stat().st_mtime)
    report['api_rlib_sha256'] = sha(rlib.read_bytes())
    probe = work/('probe.exe' if sys.platform == 'win32' else 'probe')
    run(['rustc','--edition=2024',ROOT/'tools/compare6502/word_result_proof.rs','--extern',f'actionc={rlib}','-L',f'dependency={rlib.parent}','-o',probe])
    output = run([probe, work/'api', '--pointer-setup'])
    (work/'api.jsonl').write_bytes(output)
    rows = [json.loads(line) for line in output.decode().splitlines()]
    assert len(rows) == 24
    for row in rows:
        name = row['id']
        original, before, after = args.original_api/name, args.attributed_api/name, work/'api'/name
        for artifact in ['output.debug','output.xex']:
            assert (original/artifact).read_bytes() == (before/artifact).read_bytes() == (after/artifact).read_bytes(), (name,artifact)
        old, new = summarize(before), summarize(after)
        assert old['work'] == new['work'] == json.loads((original/'manifest.json').read_text())['work'], name
        assert old['candidate_outcomes'] == new['candidate_outcomes'], name
        if name.startswith(('tn-', 'pointer-')):
            check_local_pointer_proof(old, new)
        report['api'].append(dict(id=name, before=old, after=new, full_output_sha256=sha((after/'output.debug').read_bytes()), image_sha256=sha((after/'output.xex').read_bytes())))
        print(name + ': three versions preserve full output, work and candidate outcomes', flush=True)
    pruning_probe = work/('pruning-probe.exe' if sys.platform == 'win32' else 'pruning-probe')
    run(['rustc','--edition=2024',ROOT/'tools/compare6502/late_spill_pruning.rs','--extern',f'actionc={rlib}','-L',f'dependency={rlib.parent}','-o',pruning_probe])
    pruning_output = run([pruning_probe, work/'pruning', '--tn-only'])
    (work/'pruning.jsonl').write_bytes(pruning_output)
    report['tn_pruning_recheck'] = [json.loads(line) for line in pruning_output.decode().splitlines()]
    assert len(report['tn_pruning_recheck']) == 4
    for row in report['tn_pruning_recheck']:
        name = row['id']
        for suffix in ['debug', 'xex']:
            expected_output = (work/'api'/name/f'output.{suffix}').read_bytes()
            for variant in ['before', 'after']:
                assert (work/'pruning'/f'{name}-{variant}.{suffix}').read_bytes() == expected_output
        assert row['result'] == dict(accepted=False, candidate_spills=6, unreferenced_spills=4, removed_spills=0, reason='CoreAllocation'), row
        print(name + ': pruning still rejects four eligible spills at the core allocation guard', flush=True)
    report.update(cli_artifact_comparisons=54, full_api_cases=24, versions_compared=3, trace_off_on_cases_per_version=24,
                  interpretation='Local gates of explicitly scoped candidates, including speculative materializations. Shared SSA-prefix facts and other unscoped observations remain separate; no complete preservation certificate or frame compaction is implied.')
    (work/'qualification.json').write_text(json.dumps(report,indent=2)+'\n')
    print(work/'qualification.json')


if __name__ == '__main__':
    main()
