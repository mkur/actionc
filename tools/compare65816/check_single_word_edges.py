#!/usr/bin/env python3
"""Check direct word-copy counts and exact staging-pair removal in final code."""
import argparse
import hashlib
import json
from pathlib import Path
import sys

sys.dont_write_bytecode = True
from delta import load, check_records, direct_word_edge_counts, frame_contract
from check_empty_edges import listing


def check_listing(before, after, sites):
    positions = {pc: i for i, (pc, _) in enumerate(before)}
    removed = set()
    for pc in sites:
        assert pc in positions, ('missing old edge', pc)
        i = positions[pc]
        assert i + 5 <= len(before), ('truncated old edge', pc)
        load, capture, reload, assign, jump = [code for _, code in before[i:i+5]]
        assert load[0] in (0xa3, 0xa9) and len(load) == (2 if load[0] == 0xa3 else 3)
        assert len(capture) == len(reload) == len(assign) == 2
        assert capture[0] == assign[0] == 0x83 and reload == bytes([0xa3, capture[1]])
        assert 1 <= capture[1] <= 254 and 1 <= assign[1] <= 254
        assert abs(capture[1]-assign[1]) >= 2
        if load[0] == 0xa3:
            assert 1 <= load[1] <= 254 and abs(load[1]-capture[1]) >= 2
        assert len(jump) == 4 and jump[0] == 0x5c
        assert not removed.intersection((i+1, i+2))
        removed.update((i+1, i+2))
    kept = [instruction for i, instruction in enumerate(before) if i not in removed]
    assert len(kept) == len(after), ('instruction count', len(kept), len(after))
    addresses = {old[0]: new[0] for old, new in zip(kept, after)}
    for (old_pc, old), (new_pc, new) in zip(kept, after):
        if old[0] in (0x22, 0x5c):
            target = int.from_bytes(old[1:], 'little')
            assert target not in {before[i][0] for i in removed}, 'target inside removed staging pair'
            old = old[:1]+addresses.get(target, target).to_bytes(3, 'little')
        assert old == new, ('unexpected instruction change', hex(old_pc), hex(new_pc), old.hex(), new.hex())
    return {addresses[pc] for pc in sites}


def check(before_dir, after_dir, counts, baseline):
    for field in ('measurements', 'snapshot_hashes'):
        for path, digest in baseline[field].items():
            assert hashlib.sha256(Path(path).read_bytes()).hexdigest() == digest, path
    old_manifest, old = load(before_dir)
    new_manifest, new = load(after_dir)
    predicted = direct_word_edge_counts(counts)
    check_records(old, new, {}, direct=predicted)
    assert old_manifest['cases'] == new_manifest['cases']
    for tool in ('vbcc', 'vasm', 'vlink'):
        assert old_manifest['tools'][tool]['sha256'] == new_manifest['tools'][tool]['sha256']
    indexed = {(a['case'], a['mode'], a['compiler']): a for a in old_manifest['artifacts']}
    rows = []
    for a in new_manifest['artifacts']:
        key = a['case'], a['mode'], a['compiler']
        b = indexed[key]
        for artifact in (a, b):
            for name, digest in artifact['hashes'].items():
                assert hashlib.sha256((Path(artifact['directory'])/name).read_bytes()).hexdigest() == digest
        if a['compiler'] != 'actionc':
            for name in a['hashes']:
                if name == 'code.lst':
                    # vasm writes the build directory into its source header.
                    # Compare every other byte, including its instruction text.
                    old_listing = (Path(b['directory'])/name).read_bytes()
                    new_listing = (Path(a['directory'])/name).read_bytes()
                    old_header = f'Source: "{b["directory"]}/code.asm"'.encode()
                    new_header = f'Source: "{a["directory"]}/code.asm"'.encode()
                    assert old_listing.count(old_header) == new_listing.count(new_header) == 1
                    assert old_listing.replace(old_header, new_header) == new_listing, key
                else:
                    assert a['hashes'][name] == b['hashes'][name], (key, name)
            continue
        assert [frame_contract(r) for r in a['routines']] == [frame_contract(r) for r in b['routines']]
        old_sites = set()
        for k in predicted:
            if k[:3] == key:
                assert old[k]['word_edges'] == old[k]['edge_words'] == predicted[k]
                old_sites.update(map(int, old[k]['word_edge_sites']))
        new_sites = check_listing(listing(Path(b['directory'])/'code.asm'), listing(Path(a['directory'])/'code.asm'), old_sites)
        actual = set()
        for k, record in new.items():
            if k[:3] == key:
                actual.update(map(int, record['direct_word_edge_sites']))
        assert actual == new_sites, (key, 'decoded direct sites')
        assert b['code_bytes']-a['code_bytes'] == 4*len(old_sites), key
        if not old_sites:
            assert a['hashes'] == b['hashes'], key
        rows.append(dict(case=a['case'], mode=a['mode'], direct_sites=len(old_sites)))
    for f in baseline['forecasts']:
        key = f['case'], f['mode'], 'actionc', f['vector']
        for name, field in [('bytes', 'code_bytes'), ('cycles', 'cycles'), ('stack_reads', 'stack_reads'), ('stack_writes', 'stack_writes')]:
            assert old[key][field] == f[name+'_before'] and new[key][field] == f[name+'_after'], (key, field)
        assert old[key]['peak_below_entry_s'] == new[key]['peak_below_entry_s'] == f['stack_peak']
    return dict(records=len(new), action_builds=len(rows), changed_builds=sum(r['direct_sites'] > 0 for r in rows),
                selected_static_sites=sum(r['direct_sites'] for r in rows), selected_vector_records=len(predicted),
                executed_direct_copies_per_incoming_i_state=sum(predicted.values()),
                only_staging_pairs_removed=True, forecasts_exact=True, unchanged_storage_and_guards=True, sites=rows)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('before', type=Path)
    p.add_argument('after', type=Path)
    p.add_argument('--counts', type=Path, required=True)
    p.add_argument('--baseline', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    args = p.parse_args()
    result = check(args.before, args.after, json.loads(args.counts.read_text()), json.loads(args.baseline.read_text()))
    args.output.write_text(json.dumps(result, indent=2)+'\n')
    print(f"Checked {result['records']} records, {result['action_builds']} streams and {result['selected_static_sites']} direct sites")


if __name__ == '__main__':
    main()
