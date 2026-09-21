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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('before', type=Path)
    parser.add_argument('after', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--title', default='Native word arithmetic: before / after')
    args = parser.parse_args()
    old_manifest, old = load(args.before)
    new_manifest, new = load(args.after)
    assert old_manifest['cases'] == new_manifest['cases']
    assert old.keys() == new.keys()
    for tool in ('vbcc', 'vasm', 'vlink'):
        assert old_manifest['tools'][tool]['sha256'] == new_manifest['tools'][tool]['sha256']
    preserved = ('correct', 'errors', 'result', 'args', 'incoming_stack_bytes',
                 'incoming_return_bytes', 'peak_below_entry_s', 'static_stack_check_bytes',
                 'stack_check_cycles', 'stack_check_instructions', 'stack_reads', 'stack_writes',
                 'metadata_reads', 'input_padding_reads')
    for key, before in old.items():
        after = new[key]
        for field in preserved:
            assert before[field] == after[field], (key, field, before[field], after[field])
        if before['compiler'] == 'vbcc':
            assert before == after, key
        else:
            assert after['correct'], key
            assert after['code_bytes'] <= before['code_bytes'], key
            assert after['cycles'] <= before['cycles'], key
    old_artifacts = {(a['case'], a['mode'], a['compiler']): a for a in old_manifest['artifacts']}
    for artifact in new_manifest['artifacts']:
        if artifact['compiler'] != 'actionc':
            continue
        before = old_artifacts[artifact['case'], artifact['mode'], artifact['compiler']]
        assert [frame_contract(r) for r in before['routines']] == [frame_contract(r) for r in artifact['routines']]
    lines = ['# '+args.title, '',
             'All cells are **before / after actionc**, including guards and RTL.',
             'Stack depth, byte traffic on the stack, ABI arguments, complete routine',
             'storage maps, and stack-check costs are unchanged for every vector.', '',
             'DP traffic counts byte reads plus writes; cycles are independent VM',
             'cycles. Both host build modes produce identical measurements.', '']
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
                   routine_storage_contracts_identical=True, vbcc_measurements_identical=True,
                   no_action_failures=True, no_code_size_or_cycle_regressions=True,
                   known_external_failures=[list(k) for k, v in new.items() if not v['correct']],
                   input_sha256={f'{label}/{name}': hashlib.sha256((directory/name).read_bytes()).hexdigest()
                                 for label, directory in [('before', args.before), ('after', args.after)]
                                 for name in ('manifest.json', 'debug.json', 'release.json')})
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output/'delta.md').write_text('\n'.join(lines))
    (args.output/'delta.json').write_text(json.dumps(summary, indent=2)+'\n')
    print(f'Checked {len(new)} records and all routine storage contracts: {args.output}')


if __name__ == '__main__':
    main()
