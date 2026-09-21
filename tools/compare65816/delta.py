#!/usr/bin/env python3
"""Compare matched before/after machine-code measurements and storage contracts."""
import argparse
import hashlib
import json
from pathlib import Path
import sys

sys.dont_write_bytecode = True
from report import PICKS


def load(directory):
    debug = json.loads((directory/'debug.json').read_text())
    assert debug == json.loads((directory/'release.json').read_text())
    assert debug['manifest'] == json.loads((directory/'manifest.json').read_text())
    records = {(r['case'], r['mode'], r['compiler'], r['vector']): r
               for r in debug['measurements']}
    assert len(records) == len(debug['measurements'])
    return debug['manifest'], records


def frame_contract(routine):
    return {k: v for k, v in routine.items() if k not in ('address', 'size')}


PRESERVED = ('correct', 'errors', 'result', 'args', 'incoming_stack_bytes',
             'incoming_return_bytes', 'peak_below_entry_s', 'static_stack_check_bytes',
             'stack_check_cycles', 'stack_check_instructions', 'stack_writes',
             'metadata_reads', 'input_padding_reads')


def stack_read_deltas(rows):
    expected = {}
    for row in rows:
        assert set(row) == {'case', 'mode', 'compiler', 'vector', 'delta'}, row
        assert row['compiler'] == 'actionc' and row['mode'] in ('raw', 'optimized'), row
        assert isinstance(row['case'], str) and row['case'], row
        assert type(row['vector']) is int and row['vector'] >= 0, row
        assert type(row['delta']) is int and row['delta'] > 0, row
        key = tuple(row[k] for k in ('case', 'mode', 'compiler', 'vector'))
        assert key not in expected, ('duplicate stack-read delta', key)
        expected[key] = row['delta']
    return expected


def check_records(old, new, expected):
    assert old.keys() == new.keys()
    assert expected.keys() <= new.keys(), ('unused stack-read deltas', expected.keys() - new.keys())
    for key, before in old.items():
        after = new[key]
        for field in PRESERVED:
            assert before[field] == after[field], (key, field, before[field], after[field])
        difference = after['stack_reads'] - before['stack_reads']
        assert difference == expected.get(key, 0), (key, 'stack_reads', difference, expected.get(key, 0))
        if before['compiler'] == 'vbcc':
            assert before == after, key
        else:
            assert after['correct'], key
            assert after['code_bytes'] <= before['code_bytes'], key
            assert after['cycles'] <= before['cycles'], key


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('before', type=Path)
    parser.add_argument('after', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--title', default='Native word arithmetic: before / after')
    parser.add_argument('--stack-read-deltas', type=Path,
                        help='JSON list of exact, independently predicted Action stack-read increases; default requires equality')
    args = parser.parse_args()
    old_manifest, old = load(args.before)
    new_manifest, new = load(args.after)
    assert old_manifest['cases'] == new_manifest['cases']
    for tool in ('vbcc', 'vasm', 'vlink'):
        assert old_manifest['tools'][tool]['sha256'] == new_manifest['tools'][tool]['sha256']
    delta_rows = json.loads(args.stack_read_deltas.read_text()) if args.stack_read_deltas else []
    expected = stack_read_deltas(delta_rows)
    check_records(old, new, expected)
    preserved = PRESERVED if expected else (*PRESERVED, 'stack_reads')
    old_artifacts = {(a['case'], a['mode'], a['compiler']): a for a in old_manifest['artifacts']}
    for artifact in new_manifest['artifacts']:
        if artifact['compiler'] != 'actionc':
            continue
        before = old_artifacts[artifact['case'], artifact['mode'], artifact['compiler']]
        assert [frame_contract(r) for r in before['routines']] == [frame_contract(r) for r in artifact['routines']]
    lines = ['# '+args.title, '',
             'All cells are **before / after actionc**, including guards and RTL.',
             'Stack depth, stack writes, ABI arguments, complete routine',
             'storage maps, and stack-check costs are unchanged for every vector.', '',
             'DP traffic counts byte reads plus writes; cycles are independent VM',
             'cycles. Both host build modes produce identical measurements.', '']
    if expected:
        lines.extend(['Stack reads change only by the following predeclared amounts; all',
                      'other records retain exactly their previous stack-read counts.', '',
                      '| Case | Mode | Vector | Stack reads before / after |',
                      '| --- | --- | ---: | ---: |'])
        for key in sorted(expected):
            lines.append(f'| {key[0]} | {key[1]} | {key[3]} | {old[key]["stack_reads"]} / {new[key]["stack_reads"]} |')
        lines.append('')
    else:
        lines.extend(['Stack reads are also unchanged for every vector.', ''])
    for mode in ('optimized', 'raw'):
        lines.extend([f'## {mode.title()}', '',
                      '| Kernel | Code bytes | Cycles | Stack bytes | DP traffic |',
                      '| --- | ---: | ---: | ---: | ---: |'])
        for case in new_manifest['cases']:
            name = case['id']
            key = (name, mode, 'actionc', PICKS[name])
            before, after = old[key], new[key]
            cells = [name, *[f'{before[k]} / {after[k]}' for k in
                            ('code_bytes', 'cycles', 'peak_below_entry_s')],
                     f"{before['dp_reads']+before['dp_writes']} / {after['dp_reads']+after['dp_writes']}"]
            lines.append('| '+' | '.join(cells)+' |')
        lines.append('')
    summary = dict(before_revision=old_manifest['compiler_revision'],
                   after_revision=new_manifest['compiler_revision'],
                   paired_mask_records=len(new), preserved_fields=preserved,
                   expected_stack_read_deltas=delta_rows,
                   routine_storage_contracts_identical=True, vbcc_measurements_identical=True,
                   no_action_failures=True, no_code_size_or_cycle_regressions=True,
                   known_external_failures=[list(k) for k, v in new.items() if not v['correct']],
                   input_sha256={f'{label}/{name}': hashlib.sha256((directory/name).read_bytes()).hexdigest()
                                 for label, directory in [('before', args.before), ('after', args.after)]
                                 for name in ('manifest.json', 'debug.json', 'release.json')})
    if args.stack_read_deltas:
        summary['expected_stack_read_deltas_sha256'] = hashlib.sha256(args.stack_read_deltas.read_bytes()).hexdigest()
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output/'delta.md').write_text('\n'.join(lines))
    (args.output/'delta.json').write_text(json.dumps(summary, indent=2)+'\n')
    print(f'Checked {len(new)} records and all routine storage contracts: {args.output}')


if __name__ == '__main__':
    main()
