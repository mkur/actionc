#!/usr/bin/env python3
"""Check frozen staging operand patches, complete images and every measurement."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import sys
sys.dont_write_bytecode = True
from check_empty_edges import listing
from delta import load


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def instructions(before, after, patches):
    expected = dict(before)
    assert len(expected) == len(before)
    seen = set()
    for p in patches:
        pc = p['pc']
        assert pc not in seen
        seen.add(pc)
        old, new = bytes.fromhex(p['before']), bytes.fromhex(p['after'])
        assert expected[pc] == old and len(old) == len(new) and old[0] == new[0]
        expected[pc] = new
    assert after == [(pc, expected[pc]) for pc, _ in before]


def routine(before, change):
    r = copy.deepcopy(before)
    if change is None:
        return r
    assert r['id'] == change['routine'] and r['fixed_frame'] == change['old_extent']
    saved = change['old_extent'] - change['new_extent']
    assert saved > 0 and saved % 2 == 0
    for field in ('fixed_frame', 'spill_bytes', 'local_stack_peak'):
        r[field] -= saved
    for a in r['arguments']:
        a['body_displacement'] -= saved
    assert not r['calls'] and r['whole_task_stack_bound'] is None
    return r


def image(before, change):
    d = copy.deepcopy(before)
    if change is None:
        return d
    for p in change['instruction_patches']:
        old, new = list(bytes.fromhex(p['before'])), list(bytes.fromhex(p['after']))
        segments = [s for s in d['segments'] if s['address'] <= p['pc'] < s['address']+len(s['bytes'])]
        assert len(segments) == 1
        s = segments[0]; at = p['pc'] - s['address']
        assert s['bytes'][at:at+len(old)] == old
        s['bytes'][at:at+len(old)] = new
    d['routines'] = [routine(r, change if r['id'] == change['routine'] else None) for r in d['routines']]
    return d


def check(before_dir, after_dir, frozen):
    for name, h in frozen['comparison_hashes'].items():
        assert digest(before_dir/name) == h
    bm, before = load(before_dir); am, after = load(after_dir)
    assert before.keys() == after.keys() and bm['cases'] == am['cases']
    for t in ('vbcc', 'vasm', 'vlink'):
        assert bm['tools'][t]['sha256'] == am['tools'][t]['sha256']
    artifacts = lambda m: {(a['case'], a['mode'], a['compiler']): a for a in m['artifacts']}
    ba, aa = artifacts(bm), artifacts(am)
    assert ba.keys() == aa.keys()
    changes = {(r['case'], r['mode'], 'actionc'): r for r in frozen['expected_changes']}
    assert changes.keys() <= aa.keys()
    for key, a in aa.items():
        b = ba[key]; change = changes.get(key)
        for art in (a, b):
            for name, h in art['hashes'].items():
                assert digest(Path(art['directory'])/name) == h
        if key[2] == 'vbcc':
            for name, h in a['hashes'].items():
                if name == 'code.lst':
                    old = (Path(b['directory'])/name).read_bytes()
                    assert old.replace(f'Source: "{b["directory"]}/code.asm"'.encode(), f'Source: "{a["directory"]}/code.asm"'.encode()) == (Path(a['directory'])/name).read_bytes()
                else:
                    assert b['hashes'][name] == h
            continue
        instructions(listing(Path(b['directory'])/'code.asm'), listing(Path(a['directory'])/'code.asm'), change['instruction_patches'] if change else [])
        assert json.loads(Path(a['image']).read_text()) == image(json.loads(Path(b['image']).read_text()), change), key
        expected = copy.deepcopy(b)
        expected['routines'] = [routine(r, change if change and r['id'] == change['routine'] else None) for r in b['routines']]
        if change:
            for argument in expected['arguments']:
                argument['body_displacement'] -= change['old_extent'] - change['new_extent']
        for k in ('directory', 'commands', 'image', 'hashes'):
            expected[k] = a[k]
        assert a == expected, key
        if not change:
            assert b['hashes'] == a['hashes'], key
    for key, old in before.items():
        expected = copy.deepcopy(old)
        if change := changes.get(key[:3]):
            expected['peak_below_entry_s'] -= change['old_extent'] - change['new_extent']
        assert after[key] == expected, (key, {k: (expected[k], after[key][k]) for k in expected if expected[k] != after[key][k]})
        if key[2] == 'actionc':
            assert after[key]['correct']
    return dict(records=len(after), instruction_streams=sum(k[2]=='actionc' for k in aa),
                changed_builds=len(changes), operand_patches=sum(len(c['instruction_patches']) for c in changes.values()),
                frames=[{k: c[k] for k in ('case', 'mode', 'old_extent', 'new_extent')} for c in changes.values()],
                external_failures=[list(k) for k, r in after.items() if not r['correct']])


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('before', type=Path); p.add_argument('after', type=Path)
    p.add_argument('--baseline', type=Path, required=True); p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    result = check(a.before, a.after, json.loads(a.baseline.read_text()))
    a.output.parent.mkdir(parents=True, exist_ok=True)
    a.output.write_text(json.dumps(result, indent=2)+'\n')
    print(result)
