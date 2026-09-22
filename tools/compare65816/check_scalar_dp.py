#!/usr/bin/env python3
"""Check frozen scalar DP images, all counters, address maps and external controls."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import sys
sys.dont_write_bytecode = True
from inventory_scalar_dp import transform, measurements, digest
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
        ei, ins, remap, removed = transform(bi, f['routines'])
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
        if any(not t['rejections'] for t in f['routines']):
            changed.append(list(key))
        for record in controls:
            if (record['case'], record['mode'], 'actionc') == key:
                for proof in record['sites']:
                    for field in ('pc', 'target'):
                        if field in proof:
                            proof[field] = remap(proof[field])
        for projection in f['projections']:
            record_key = (*key, projection['vector'])
            e = measurements(before[record_key], f['routines'], remap, removed, bi)
            assert canonical(e) == projection['forecast_sha256'], ('counter forecast changed', record_key)
            assert e == after[record_key], (record_key, {k: (v, after[record_key].get(k)) for k, v in e.items() if v != after[record_key].get(k)})
            assert e['correct']
            # Instruction bytes and widths are frozen and independently decoded;
            # dynamic counts now come from the executed, changed image.
            resident = {'read': 0, 'write': 0}
            for t in f['routines']:
                for site in t.get('accesses', []):
                    resident[site['access']] += after[record_key]['instruction_sites'].get(str(remap(site['pc'])), 0) * site['width']
            traffic.append(dict(case=key[0], mode=key[1], vector=projection['vector'],
                resident_dp_reads=resident['read'], resident_dp_writes=resident['write'],
                selector_dp_reads=e['dp_reads']-resident['read'], selector_dp_writes=e['dp_writes']-resident['write'], metadata_reads=e['metadata_reads']))
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


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('before', type=Path); p.add_argument('after', type=Path)
    p.add_argument('--inventory', type=Path, required=True); p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    result = check(a.before, a.after, json.loads(a.inventory.read_text()))
    a.output.parent.mkdir(parents=True, exist_ok=True)
    a.output.write_text(json.dumps(result, indent=2)+'\n')
    print({k: v for k, v in result.items() if k != 'traffic'})
