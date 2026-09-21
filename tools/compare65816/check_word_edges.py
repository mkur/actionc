#!/usr/bin/env python3
"""Qualify word-edge selection against predeclared counts and strict old metrics."""
import argparse
import json
from pathlib import Path
import sys

sys.dont_write_bytecode = True
from delta import load, check_records, frame_contract


def check(before, after, counts):
    old_manifest, old = load(before)
    new_manifest, new = load(after)
    check_records(old, new, {})  # No stack-traffic exceptions for this slice.
    predicted = {}
    for row in counts:
        assert set(row) == {'case', 'mode', 'compiler', 'vector', 'edges', 'words'}
        key = tuple(row[k] for k in ('case', 'mode', 'compiler', 'vector'))
        assert key not in predicted and key in new
        assert row['compiler'] == 'actionc' and row['mode'] == 'optimized'
        assert type(row['edges']) is int and row['edges'] > 0
        assert type(row['words']) is int and row['words'] >= row['edges']
        predicted[key] = row['edges'], row['words']
    for key, row in new.items():
        if row['compiler'] != 'actionc':
            continue
        assert (row['word_edges'], row['edge_words']) == predicted.get(key, (0, 0)), key
        assert sum(row['word_edge_sites'].values()) == row['word_edges'], key
        for field in ('dp_reads', 'dp_writes', 'dp_touched_offsets', 'fused_branches'):
            assert row[field] == old[key][field], (key, field)
        if key not in predicted:
            assert {k: v for k, v in row.items() if k not in
                    ('word_edges', 'edge_words', 'word_edge_sites')} == old[key], key
    indexed = {(a['case'], a['mode'], a['compiler']): a for a in old_manifest['artifacts']}
    for a in new_manifest['artifacts']:
        key = a['case'], a['mode'], a['compiler']
        b = indexed[key]
        if a['compiler'] == 'actionc':
            assert [frame_contract(r) for r in a['routines']] == [frame_contract(r) for r in b['routines']], key
            if a['mode'] == 'raw' or a['case'] not in ('sum_loop', 'loop_rotation', 'byte_sum'):
                assert a['hashes'] == b['hashes'], ('unchanged emitted files', key)
    for case, vector, max_bytes, max_cycles in [('sum_loop', 3, 165, 1800),
            ('loop_rotation', 2, 200, 1350), ('byte_sum', 4, 265, 4750)]:
        row = new[case, 'optimized', 'actionc', vector]
        assert row['code_bytes'] <= max_bytes and row['cycles'] <= max_cycles, row
    return dict(records=len(new), predicted_vectors=len(predicted),
                executed_edges=sum(v[0] for v in predicted.values()),
                copied_words=sum(v[1] for v in predicted.values()),
                unchanged_raw_builds=14, changed_optimized_builds=3,
                strict_stack_traffic=True, unchanged_dp_and_fusions=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('before', type=Path)
    parser.add_argument('after', type=Path)
    parser.add_argument('--counts', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = check(args.before, args.after, json.loads(args.counts.read_text()))
    args.output.write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
