#!/usr/bin/env python3
"""Compare ordinary word-result proofs with a frozen pre-change API corpus.

Build word_result_proof.rs against the baseline rlib and run it once to capture
--baseline-api before rebuilding the compiler. Debug snapshots cover every
CodegenOutput field; hashes are diagnostic artifacts, not compiler proofs.
"""
import argparse
import collections
import json
import sys
from pathlib import Path
import tempfile

from core_layout_census import ROOT, run, sha

FAMILY = 'known-callee-word-result-placement'


def summarize(directory):
    manifest = json.loads((directory / 'manifest.json').read_text())
    assert manifest['status'] == 'complete'
    assert manifest['core_layout_census']['status'] == 'complete'
    counts = collections.Counter()
    per_routine = collections.defaultdict(collections.Counter)
    names = {}
    for line in (directory / 'events.jsonl').open():
        event = json.loads(line)
        data = event['data']
        if event['kind'] == 'snapshot':
            names.update({r['id']: r['name'] for r in data.get('routines', [])})
        if event['kind'] != 'core-layout-round':
            continue
        applied = data['applied']
        if not any(p['family'] == FAMILY for p in applied):
            continue
        assert all(p['family'] == FAMILY for p in applied)
        counters = [counts, per_routine[data['routine']]]
        for counter in counters:
            counter['applied_rounds'] += 1
        for entry in data['observations']:
            observation = entry['observation']
            predicate = observation['predicate']
            for counter in counters:
                counter[predicate] += entry['evaluations']
                if predicate == 'ordinary-range-outside-zero-page':
                    counter['structural_true' if observation['result'] else 'structural_false'] += entry['evaluations']
    return dict(work=manifest['work'], bytes=manifest['output_sizes']['load_file_bytes'],
                status=manifest['core_layout_census']['status'], applied_family_round_envelopes=dict(counts),
                by_routine=[dict(id=r, name=names.get(r), counts=dict(c)) for r, c in sorted(per_routine.items())])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline-api', required=True, type=Path)
    parser.add_argument('--baseline-revision', default='9149ed66')
    parser.add_argument('--compiler', type=Path, default=ROOT / 'build/checked-arena/shared-target/debug/actionc-emit')
    parser.add_argument('--api-rlib', type=Path)
    parser.add_argument('--output', type=Path, default=ROOT / 'build/mir6502-word-result-proof/qualification')
    args = parser.parse_args()
    compiler = args.compiler.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix='run-', dir=args.output)).resolve()
    frozen = json.loads((ROOT / 'docs/benchmarks/6502-late-spills/qualification.json').read_text())
    catalog = json.loads((ROOT / 'docs/benchmarks/6502-phase-tracing/baseline.json').read_text())
    expected = {c['id']: c for c in frozen['cli_cases']}
    report = dict(baseline_revision=args.baseline_revision,
                  implementation_revision=run(['git', 'rev-parse', 'HEAD']).decode().strip(),
                  compiler_sha256=sha(compiler.read_bytes()), cli_cases=[], api=[])
    for case in catalog['cases']:
        assert sha((ROOT / case['source']).read_bytes().replace(b'\r\n', b'\n')) == case['source_sha256']
        command = [compiler, '--profile', case['profile'], '--backend', 'mir6502', '--runtime', case['runtime'], '--origin', case['origin']]
        artifacts = {}
        for suffix, mode in [('xex', '--emit-load'), ('asm', '--emit-listing'), ('map', '--emit-map')]:
            data = run(command + [mode, case['source']])
            assert sha(data) == expected[case['id']]['artifacts'][suffix]['sha256'], (case['id'], suffix)
            artifacts[suffix] = dict(sha256=sha(data), bytes=len(data))
        report['cli_cases'].append(dict(id=case['id'], artifacts=artifacts))
        print(case['id'] + ': frozen artifacts unchanged', flush=True)
    rlib = args.api_rlib or max((compiler.parent / 'deps').glob('libactionc-*.rlib'), key=lambda p: p.stat().st_mtime)
    report['api_rlib_sha256'] = sha(rlib.read_bytes())
    probe = work / ('probe.exe' if sys.platform == 'win32' else 'probe')
    run(['rustc', '--edition=2024', ROOT / 'tools/compare6502/word_result_proof.rs', '--extern', f'actionc={rlib}', '-L', f'dependency={rlib.parent}', '-o', probe])
    output = run([probe, work / 'api'])
    (work / 'api.jsonl').write_bytes(output)
    rows = [json.loads(line) for line in output.decode().splitlines()]
    assert len(rows) == 20
    for row in rows:
        name = row['id']
        before = args.baseline_api / name
        after = work / 'api' / name
        for artifact in ['output.xex', 'output.debug']:
            assert (before / artifact).read_bytes() == (after / artifact).read_bytes(), (name, artifact)
        old, new = summarize(before), summarize(after)
        assert old['work'] == new['work'], name
        report['api'].append(dict(id=name, before=old, after=new,
            image_sha256=sha((after / 'output.xex').read_bytes()),
            full_output_debug_sha256=sha((after / 'output.debug').read_bytes())))
        print(name + ': full output and work unchanged', flush=True)
    report.update(cli_artifact_comparisons=54, full_api_baseline_comparisons=20, full_api_trace_off_on_comparisons=20,
                  interpretation='Counts are local applied discovery-round envelopes, including speculative materializations and rejected candidates. Structural observations are not numeric queries or persistent preservation witnesses.')
    report['source_sha256_lf'] = {p: sha((ROOT / p).read_bytes().replace(b'\r\n', b'\n')) for p in [
        'samples/tn/modern/TN.ACT', 'fixtures/runtime/word_load_sharing.act',
        'fixtures/runtime/argument_staging.act', 'fixtures/runtime/pointer_invariant_loop.act',
        'fixtures/runtime/core_layout_census.act']}
    (work / 'qualification.json').write_text(json.dumps(report, indent=2) + '\n')
    print(work / 'qualification.json')


if __name__ == '__main__':
    main()
