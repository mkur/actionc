#!/usr/bin/env python3
"""Prove empty-edge cleanup removes only redundant SEP/REP instructions."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import sys

sys.dont_write_bytecode = True
from delta import load, check_records, frame_contract


def listing(path):
    result = []
    for line in path.read_text().splitlines():
        match = re.fullmatch(r'([0-9A-F]{6})\s+((?:[0-9A-F]{2}\s+)+)\S.*', line)
        assert match, (path, line)
        result.append((int(match[1], 16), bytes.fromhex(match[2])))
    return result


def check_listing(before, after):
    # True-edge labels are JML targets and invalidate local mode knowledge.
    # Their REP remains. Other empty edges are emitted after known A16.
    targets = {int.from_bytes(code[1:], 'little') for _, code in before if code[0] == 0x5c}
    removed = set()
    edges = 0
    for i in range(len(before)-2):
        pc, code = before[i]
        if code == b'\xe2\x20' and before[i+1][1] == b'\xc2\x20' and before[i+2][1][0] == 0x5c:
            edges += 1
            removed.add(i)
            if pc not in targets:
                removed.add(i+1)
    kept = [instruction for i, instruction in enumerate(before) if i not in removed]
    assert len(kept) == len(after), ('instruction count', len(kept), len(after))
    addresses = {old[0]: new[0] for old, new in zip(kept, after)}
    # A target at an eliminated mode instruction now names the next retained one.
    for i in sorted(removed, reverse=True):
        addresses[before[i][0]] = addresses[before[i+1][0]]
    for (old_pc, old), (new_pc, new) in zip(kept, after):
        if old[0] in (0x22, 0x5c):  # typed JSL/JML targets relocate with text
            target = int.from_bytes(old[1:], 'little')
            old = old[:1]+addresses.get(target, target).to_bytes(3, 'little')
        assert old == new, ('unexpected instruction change', hex(old_pc), hex(new_pc), old.hex(), new.hex())
    return edges, len(removed)


def check(before_dir, after_dir, baseline):
    old_manifest, old = load(before_dir)
    new_manifest, new = load(after_dir)
    check_records(old, new, {})  # Retain strict stack/guard/ABI/correctness checks.
    assert old_manifest['cases'] == new_manifest['cases']
    rows = []
    indexed = {(a['case'], a['mode'], a['compiler']): a for a in old_manifest['artifacts']}
    for a in new_manifest['artifacts']:
        if a['compiler'] != 'actionc':
            continue
        key = a['case'], a['mode'], a['compiler']
        b = indexed[key]
        for artifact in (a, b):
            for name, sha in artifact['hashes'].items():
                assert hashlib.sha256((Path(artifact['directory'])/name).read_bytes()).hexdigest() == sha
        assert [frame_contract(r) for r in a['routines']] == [frame_contract(r) for r in b['routines']]
        edges, removed = check_listing(listing(Path(b['directory'])/'code.asm'), listing(Path(a['directory'])/'code.asm'))
        assert b['code_bytes']-a['code_bytes'] == 2*removed, key
        if removed == 0:
            assert a['hashes'] == b['hashes'], key
        rows.append(dict(case=a['case'], mode=a['mode'], empty_sites=edges, removed_instructions=removed))
    for key, after in new.items():
        before = old[key]
        if after['compiler'] != 'actionc':
            continue
        for field in ('dp_reads', 'dp_writes', 'dp_touched_offsets', 'fused_branches', 'word_edges', 'edge_words'):
            assert before[field] == after[field], (key, field)
        for field in ('fused_branch_sites', 'word_edge_sites'):
            assert sorted(before[field].values()) == sorted(after[field].values()), (key, field)
        removed = before['instructions']-after['instructions']
        assert removed >= 0 and before['cycles']-after['cycles'] == 3*removed, key
    for forecast in baseline['forecasts']:
        key = forecast['case'], forecast['mode'], 'actionc', forecast['vector']
        for name, field in [('bytes', 'code_bytes'), ('cycles', 'cycles')]:
            assert old[key][field] == forecast[name+'_before'], key
            assert new[key][field] == forecast[name+'_after'], key
    return dict(records=len(new), action_builds=len(rows),
                changed_builds=sum(r['removed_instructions'] > 0 for r in rows),
                removed_static_instructions=sum(r['removed_instructions'] for r in rows),
                only_empty_edge_mode_changes=True, unchanged_stack_dp_and_guards=True,
                unchanged_fusion_and_word_copy_counts=True, forecasts_exact=True, sites=rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('before', type=Path)
    parser.add_argument('after', type=Path)
    parser.add_argument('--baseline', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = check(args.before, args.after, json.loads(args.baseline.read_text()))
    args.output.write_text(json.dumps(result, indent=2)+'\n')
    print(f"Checked {result['records']} records and {result['action_builds']} instruction streams")


if __name__ == '__main__':
    main()
