#!/usr/bin/env python3
"""Check frozen loop X images, all counters, address maps and external controls."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import sys
import tempfile
sys.dont_write_bytecode = True
from loop_x import transform, measurement, digest
from check_selective_staging import all_instructions
from check_empty_edges import listing
from delta import load


def canonical(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def check(before_dir, after_dir, frozen):
    for name, h in frozen['comparison_hashes'].items():
        assert digest(before_dir/name) == h
    bm, before = load(before_dir)
    am, after = load(after_dir)
    assert before.keys() == after.keys() and bm['cases'] == am['cases']
    for name in ('vbcc', 'vasm', 'vlink'):
        assert bm['tools'][name]['sha256'] == am['tools'][name]['sha256']
    artifacts = lambda m: {(a['case'], a['mode'], a['compiler']): a for a in m['artifacts']}
    ba, aa = artifacts(bm), artifacts(am)
    assert ba.keys() == aa.keys()
    forecasts = {(b['case'], b['mode'], 'actionc'): b for b in frozen['builds']}
    controls = copy.deepcopy(json.loads((before_dir/'debug.json').read_text())['control'])
    traffic = []
    changed = []
    for key, a in aa.items():
        b = ba[key]
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
        bi, ai = [json.loads(Path(v['image']).read_text()) for v in (b, a)]
        f = forecasts[key]
        t = f['selected']
        ei, ins, remap, refresh = transform(bi, t) if t else (bi, all_instructions(bi), lambda p:p, {})
        assert canonical(ei) == f['expected_image_sha256'], ('changed forecast', key)
        assert ai == ei, ('image differs from frozen transform', key)
        assert all_instructions(ai) == ins
        assert listing(Path(a['directory'])/'code.asm') == [(pc, code) for pc, code in ins if any(remap(lo) <= pc < remap(hi) for lo, hi in b['code_ranges'])]
        expected = copy.deepcopy(b)
        expected['entry'] = remap(b['entry'])
        for field in ('code_ranges', 'guard_ranges'):
            expected[field] = [[remap(lo), remap(hi)] for lo, hi in b[field]]
        expected['code_bytes'] = sum(hi-lo for lo, hi in expected['code_ranges'])
        byid = {r['id']: r for r in ei['routines']}
        expected['routines'] = [byid[r['id']] for r in b['routines']]
        expected['arguments'] = next(r['arguments'] for r in ei['routines'] if r['address'] == expected['entry'])
        for field in ('directory', 'commands', 'image', 'hashes'):
            expected[field] = a[field]
        assert expected == a, ('manifest', key)
        if t:
            changed.append(list(key))
        for record in controls:
            if (record['case'], record['mode'], 'actionc') == key:
                for proof in record['sites']:
                    if t and proof['pc'] == t['branch_pc']:
                        assert proof['predicate'] == 0xb0
                        proof['predicate'] = 0x90
                    for field in ('pc', 'target'):
                        if field in proof:
                            proof[field] = remap(proof[field])
        for projection in f['projections']:
            record_key = (*key, projection['vector'])
            e = measurement(before[record_key], t, remap, refresh)
            assert e == projection['forecast'], ('counter forecast changed', record_key)
            assert e == after[record_key], (record_key, {k: (v, after[record_key].get(k)) for k, v in e.items() if v != after[record_key].get(k)})
            assert e['correct']
            if t:
                traffic.append(dict(case=key[0], mode=key[1], vector=projection['vector'],
                    before={k:before[record_key][k] for k in ('code_bytes','cycles','instructions','dp_reads','dp_writes','stack_reads','stack_writes','peak_below_entry_s')},
                    after={k:e[k] for k in ('code_bytes','cycles','instructions','dp_reads','dp_writes','stack_reads','stack_writes','peak_below_entry_s')},
                    x_forwarded_loads=e['x_forwarded_loads']))
    for key, old in before.items():
        if key[2] == 'vbcc':
            assert old == after[key]
    assert json.loads((after_dir/'debug.json').read_text())['control'] == controls
    failures = [list(k) for k, r in after.items() if not r['correct']]
    assert failures == [list(k) for k, r in before.items() if not r['correct']]
    assert failures == [['unlink', 'optimized', 'vbcc', 0]]
    return dict(records=len(after), complete_action_images=len(forecasts),
        changed_builds=changed, unchanged_action_builds=len(forecasts)-len(changed),
        frozen_images_and_all_vector_forecasts_exact=True,
        forwarding_and_copy_counters_preserved=True, guard_algorithm_and_abi_preserved=True,
        traffic=traffic, external_failures=failures)


def negative_controls(before_dir, after_dir, frozen):
    """Corrupt paired records without touching executable artifacts or baselines."""
    actual = json.loads((after_dir/'debug.json').read_text())
    rejected = []
    with tempfile.TemporaryDirectory(prefix='loop-x-counter-controls-') as directory:
        directory = Path(directory)
        (directory/'manifest.json').write_bytes((after_dir/'manifest.json').read_bytes())
        for field in ('cycles', 'dp_reads', 'dp_writes', 'x_forwarded_loads',
                      'selective_word_edges', 'coalesced_word_copies', 'correct', 'predicate'):
            changed = copy.deepcopy(actual)
            row = next(r for r in changed['measurements'] if
                       (r['case'], r['mode'], r['compiler']) == ('loop_rotation', 'optimized', 'actionc'))
            if field == 'predicate':
                sites = next(r['sites'] for r in changed['control'] if
                             (r['case'], r['mode']) == ('loop_rotation', 'optimized'))
                next(r for r in sites if 'predicate' in r)['predicate'] ^= 0x20
            elif field == 'correct':
                row[field] = False
            else:
                row[field] += 1
            for profile in ('debug', 'release'):
                (directory/(profile+'.json')).write_text(json.dumps(changed))
            try:
                check(before_dir, directory, frozen)
            except AssertionError:
                rejected.append(field)
            else:
                raise AssertionError(('accepted corrupt counter', field))
    return rejected


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('before', type=Path); p.add_argument('after', type=Path)
    p.add_argument('--inventory', type=Path, required=True); p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    frozen = json.loads(a.inventory.read_text())
    result = check(a.before, a.after, frozen)
    result['rejected_counter_mutations'] = negative_controls(a.before, a.after, frozen)
    a.output.parent.mkdir(parents=True, exist_ok=True)
    a.output.write_text(json.dumps(result, indent=2)+'\n')
    print({k: v for k, v in result.items() if k != 'traffic'})
