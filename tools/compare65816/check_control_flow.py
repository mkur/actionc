#!/usr/bin/env python3
"""Freeze control-flow predictions; check exact final bytes and executed deltas."""
import argparse
import hashlib
import json
from pathlib import Path
import sys
sys.dont_write_bytecode = True
from check_empty_edges import listing
from delta import load, frame_contract


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def artifacts(manifest):
    return {(a['case'], a['mode'], a['compiler']): a for a in manifest['artifacts']}


def select(sites, slice_name):
    candidates = [dict(s) for s in sites if s['slice'] == slice_name]
    if slice_name == '3c':
        # Independent fixed-point prediction, using absolute original positions.
        selected = set()
        while True:
            additions = set()
            for s in candidates:
                pc, target = s['pc'], s['target']
                proposed = selected | {pc}
                def position(p):
                    return p - 4 * sum(at + 6 <= p for at in proposed)
                if -128 <= position(target) - (position(pc) + 2) <= 127:
                    additions.add(pc)
            updated = selected | additions
            if updated == selected:
                break
            selected = updated
        for s in candidates:
            s['selected'] = s['pc'] in selected
    return candidates


def freeze(directory, slice_name):
    manifest, records = load(directory)
    raw = json.loads((directory / 'debug.json').read_text())
    selected = []
    for group in raw['control']:
        selected.append(dict(case=group['case'], mode=group['mode'],
                             sites=select(group['sites'], slice_name)))
    assert len(selected) == 28
    hashes = {str(directory / name): digest(directory / name)
              for name in ('manifest.json', 'debug.json', 'release.json')}
    for a in manifest['artifacts']:
        for name, sha in a['hashes'].items():
            path = str(Path(a['directory']) / name)
            assert digest(path) == sha, path
            hashes[path] = sha
    return dict(slice=slice_name, baseline_hashes=hashes, inventory=selected,
                records=len(records))


def transform(before, after, sites, slice_name):
    """Validate the entire instruction stream and return every old PC's mapping."""
    positions = {pc: i for i, (pc, _) in enumerate(before)}
    assert len(positions) == len(before)
    removed, shortened = set(), {}
    for site in sites:
        if not site['selected']:
            continue
        pc = site['pc']
        assert pc in positions, ('missing selected instruction', pc)
        i = positions[pc]
        code = before[i][1]
        if slice_name == '3a':
            assert code == b'\xc2\x20', ('entry REP', pc, code)
        elif slice_name == '3b':
            assert code[0] == 0x5c and len(code) == 4
            assert int.from_bytes(code[1:], 'little') == pc + 4 == site['target']
        else:
            assert len(code) == 2 and code == bytes([site['predicate'] ^ 0x20, 4])
            assert before[i+1] == (pc+2, b'\x5c' + site['target'].to_bytes(3, 'little'))
            assert i not in shortened, 'duplicate conditional'
            shortened[i] = site
            i += 1
        assert i not in removed, 'duplicate removal'
        removed.add(i)
    kept = [(i, ins) for i, ins in enumerate(before) if i not in removed]
    assert len(kept) == len(after), ('instruction count', len(kept), len(after))
    mapping = {old[0]: new[0] for (_, old), new in zip(kept, after)}
    # A label at a removed REP/JML still names the next retained instruction.
    for i in sorted(removed, reverse=True):
        assert i + 1 < len(before)
        mapping[before[i][0]] = mapping[before[i+1][0]]
    into_operands = {before[i][0] for i in removed} if slice_name == '3c' else set()
    for (i, (pc, old)), (new_pc, new) in zip(kept, after):
        if i in shortened:
            site = shortened[i]
            target = mapping[site['target']]
            delta = target - new_pc - 2
            assert -128 <= delta <= 127
            expected = bytes([site['predicate'], delta & 255])
        elif old[0] in (0x22, 0x5c, 0xaf, 0x8f):
            target = int.from_bytes(old[1:], 'little')
            assert target not in into_operands, ('entry inside shortened transfer', target)
            expected = old[:1] + mapping.get(target, target).to_bytes(3, 'little')
        elif old[0] in (0x10, 0x30, 0x90, 0xb0, 0xd0, 0xf0):
            target = pc + 2 + int.from_bytes(old[1:], 'little', signed=True)
            assert target not in into_operands
            delta = mapping[target] - new_pc - 2
            assert -128 <= delta <= 127
            expected = old[:1] + bytes([delta & 255])
        elif old[0] == 0x62:
            continuation = pc + 3 + int.from_bytes(old[1:], 'little', signed=True) + 1
            delta = mapping[continuation] - 1 - (new_pc + 3)
            expected = old[:1] + delta.to_bytes(2, 'little', signed=True)
        else:
            expected = old
        assert new == expected, ('unexpected instruction', hex(pc), hex(new_pc), old.hex(), new.hex(), expected.hex())
    return mapping, {before[i][0] for i in removed}


def check(before_dir, after_dir, frozen):
    for path, sha in frozen['baseline_hashes'].items():
        assert digest(path) == sha, ('changed frozen baseline', path)
    bm, before = load(before_dir)
    am, after = load(after_dir)
    assert before.keys() == after.keys()
    assert bm['cases'] == am['cases']
    for tool in ('vbcc', 'vasm', 'vlink'):
        assert bm['tools'][tool]['sha256'] == am['tools'][tool]['sha256']
    ba, aa = artifacts(bm), artifacts(am)
    for a in (*ba.values(), *aa.values()):
        for name, sha in a['hashes'].items():
            assert digest(Path(a['directory']) / name) == sha
    inventory = {(g['case'],g['mode'],'actionc'):g['sites'] for g in frozen['inventory']}
    assert len(inventory) == 28
    maps, removed = {}, {}
    for key, sites in inventory.items():
        maps[key], removed[key] = transform(listing(Path(ba[key]['directory'])/'code.asm'),
                                            listing(Path(aa[key]['directory'])/'code.asm'), sites, frozen['slice'])
        assert [frame_contract(r) for r in ba[key]['routines']] == [frame_contract(r) for r in aa[key]['routines']]
    deltas = []
    mapped_fields = ('instruction_sites', 'fused_branch_sites', 'word_edge_sites',
                     'direct_word_edge_sites', 'forwarded_word_load_sites')
    for key, old in before.items():
        new = after[key]
        if key[2] == 'vbcc':
            assert old == new, ('external output changed', key)
            continue
        assert old.keys() == new.keys()
        build = key[:3]
        mapping = maps[build]
        sites = [s for s in inventory[build] if s['selected']]
        counts = {int(pc): n for pc,n in old['instruction_sites'].items()}
        lost_instructions = sum(counts.get(pc,0) for pc in removed[build])
        if frozen['slice'] == '3c':
            lost_cycles = sum(counts.get(s['pc'],0) + 2*counts.get(s['pc']+2,0) for s in sites)
        else:
            lost_cycles = lost_instructions * (3 if frozen['slice'] == '3a' else 4)
        lost_bytes = len(sites) * (2 if frozen['slice'] == '3a' else 4)
        for field in old:
            if field in mapped_fields:
                expected = {}
                for pc, n in old[field].items():
                    pc = int(pc)
                    if field == 'instruction_sites' and pc in removed[build]:
                        continue
                    target = str(mapping[pc])
                    assert target not in expected, ('merged counted sites', key, field, target)
                    expected[target] = n
                assert new[field] == expected, (key,field,expected,new[field])
            elif field in ('instructions','cycles','code_bytes'):
                delta = dict(instructions=lost_instructions,cycles=lost_cycles,code_bytes=lost_bytes)[field]
                assert old[field] - new[field] == delta, (key,field,old[field],new[field],delta)
            else:
                assert old[field] == new[field], (key,field,old[field],new[field])
        assert new['correct']
        deltas.append(dict(case=key[0],mode=key[1],vector=key[3],bytes=lost_bytes,
                           instructions=lost_instructions,cycles=lost_cycles))
    return dict(slice=frozen['slice'],records=len(after),static_sites=sum(len(v) for v in removed.values()),
                exact_instruction_streams=28,deltas=deltas,
                external_failures=[list(k) for k,v in after.items() if not v['correct']])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command',required=True)
    f = sub.add_parser('freeze');f.add_argument('before',type=Path)
    f.add_argument('--slice',choices=['3a','3b','3c'],required=True)
    c = sub.add_parser('check');c.add_argument('before',type=Path);c.add_argument('after',type=Path)
    c.add_argument('--baseline',type=Path,required=True)
    for p in (f,c): p.add_argument('--output',type=Path,required=True)
    args = parser.parse_args()
    result = freeze(args.before,args.slice) if args.command == 'freeze' else check(args.before,args.after,json.loads(args.baseline.read_text()))
    args.output.parent.mkdir(parents=True,exist_ok=True)
    args.output.write_text(json.dumps(result,indent=2)+'\n')
    print(args.output)


if __name__ == '__main__':
    main()
