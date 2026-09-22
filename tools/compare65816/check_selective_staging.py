#!/usr/bin/env python3
"""Prove the frozen selective-copy transform, complete images and all measurements."""
import argparse
import copy
import json
from pathlib import Path
import sys
sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from disassemble65816 import disassemble
from check_empty_edges import listing
from check_staging_reservations import digest, routine as shrink_frame
from delta import load


EXTRA = ('selective_word_edges', 'selective_edge_words', 'selective_staged_words',
         'selective_direct_words', 'selective_word_edge_sites')


def address(pc, site):
    start, end = site['first_copy_pc'], site['old_transfer_pc']
    assert not start < pc < end, ('reference into transformed copy body', pc)
    return pc - (end - site['new_transfer_pc']) if pc >= end else pc


def relocate(pc, code, new_pc, remap):
    if code[0] in (0x22, 0x5c, 0xaf, 0x8f):
        target = int.from_bytes(code[1:], 'little')
        return code[:1] + remap(target).to_bytes(3, 'little')
    if code[0] in (0x10, 0x30, 0x90, 0xb0, 0xd0, 0xf0):
        target = pc + 2 + int.from_bytes(code[1:], 'little', signed=True)
        return code[:1] + (remap(target) - new_pc - 2).to_bytes(1, 'little', signed=True)
    if code[0] == 0x62:
        resume = pc + 4 + int.from_bytes(code[1:], 'little', signed=True)
        return code[:1] + (remap(resume) - new_pc - 4).to_bytes(2, 'little', signed=True)
    return code


def instructions(before, site, text_ranges):
    """Construct expected instructions without looking at new compiler output."""
    start, end = site['first_copy_pc'], site['old_transfer_pc']
    old = dict(before)
    assert len(old) == len(before)
    window = [(pc, b) for pc, b in before if start <= pc < end]
    assert [pc for pc, _ in window] == list(range(start, end, 2))
    assert b''.join(b for _, b in window).hex() == site['before_copy_bytes']
    assert site['captured_move_indices'] == [1]
    assert site['source_offsets'] == [6, 10, 12] and site['destination_offsets'] == [10, 6, 8]
    assert site['new_capture_slots'] == [dict(index=0, offset=14, width=2)]
    assert (end, site['new_transfer_pc'], site['target_pc']) == (start+24, start+16, 0x10045)
    # The retained source loads and destination stores are identified in the old
    # stream. Drop exactly two stage stores/reloads; remap the retained capture.
    order = [2, 3, 0, 7, 8, 9, 4, 11]
    removed = {window[i][0] for i in [1, 5, 6, 10]}
    patches = {
        0x10015: ('e91400', 'e91000'), 0x10026: ('a91400', 'a91000'),
        0x1002e: ('a318', 'a314'), 0x1008e: ('691400', '691000'),
        start+6: ('8310', '830e'), start+16: ('a310', 'a30e'),
    }
    for pc, (b, _) in patches.items():
        assert old[pc].hex() == b
    remap = lambda pc: address(pc, site) if any(lo <= pc <= hi for lo, hi in text_ranges) else pc
    expected, mapping = [], {}
    for pc, b in before:
        if start <= pc < end:
            if pc != start:
                continue
            for j, i in enumerate(order):
                at, code = window[i]
                code = bytes.fromhex(patches[at][1]) if at in patches else code
                mapping[at] = start + 2*j
                expected.append((mapping[at], code))
            continue
        new_pc = remap(pc)
        code = bytes.fromhex(patches[pc][1]) if pc in patches else b
        code = relocate(pc, code, new_pc, remap)
        mapping[pc] = new_pc
        expected.append((new_pc, code))
    assert b''.join(b for pc, b in expected if start <= pc < site['new_transfer_pc']).hex() == site['forecast_copy_bytes']
    return expected, mapping, removed, remap


def image(before, expected, remap, site):
    result = copy.deepcopy(before)
    result['entry'] = remap(result['entry'])
    for segment in result['segments']:
        if not segment['executable']:
            continue
        lo, hi = segment['address'], segment['address'] + len(segment['bytes'])
        new_lo, new_hi = remap(lo), remap(hi)
        code = [(pc, b) for pc, b in expected if new_lo <= pc < new_hi]
        cursor = new_lo
        for pc, b in code:
            assert pc == cursor
            cursor += len(b)
        assert cursor == new_hi
        segment['address'] = new_lo
        segment['bytes'] = list(b''.join(b for _, b in code))
    result['routines'] = [routine(r, remap, site) for r in before['routines']]
    return result


def routine(r, remap, site):
    change = dict(routine=site['routine'], old_extent=site['old_frame']['extent'],
                  new_extent=site['forecast_frame']['extent']) if r['id'] == site['routine'] else None
    result = shrink_frame(r, change)
    result['address'] = remap(r['address'])
    result['size'] = remap(r['address']+r['size']) - result['address']
    assert not result['calls'], 'frozen changed image has no calls'
    return result


def all_instructions(image):
    # Include every executable segment, especially the later uncounted driver.
    import re
    result = []
    for line in disassemble(image).splitlines():
        m = re.fullmatch(r'([0-9A-F]{6})\s+((?:[0-9A-F]{2}\s+)+)\S.*', line)
        assert m, line
        result.append((int(m[1],16),bytes.fromhex(m[2])))
    return result


def measurements(old, site, forecasts, mapping, removed, remap):
    result = copy.deepcopy(old)
    result.update({k: ({} if k.endswith('_sites') else 0) for k in EXTRA})
    if site is None:
        return result
    forecast = forecasts[old['vector']]
    for field, value in forecast['before'].items():
        assert old[field] == value
        result[field] = forecast['forecast'][field]
    result.update(forecast['expected_selective_counters'])
    result['selective_word_edge_sites'] = {str(site['first_copy_pc']): forecast['edge_executions']}
    for field, value in old.items():
        if field == 'instruction_sites':
            result[field] = {str(mapping[int(pc)]): n for pc,n in value.items() if int(pc) not in removed}
        elif field.endswith('_sites'):
            result[field] = {str(remap(int(pc))): n for pc,n in value.items()}
    return result


def check(before_dir, after_dir, frozen):
    for name, h in frozen['comparison_hashes'].items():
        assert digest(before_dir/name) == h
    bm, before = load(before_dir); am, after = load(after_dir)
    assert before.keys() == after.keys() and bm['cases'] == am['cases']
    for t in ('vbcc', 'vasm', 'vlink'):
        assert bm['tools'][t]['sha256'] == am['tools'][t]['sha256']
    artifacts = lambda m: {(a['case'],a['mode'],a['compiler']):a for a in m['artifacts']}
    ba, aa = artifacts(bm), artifacts(am)
    assert ba.keys() == aa.keys()
    s = frozen['selected_site']; changed = (s['case'],s['mode'],'actionc')
    mapping, removed, remap = {}, set(), lambda p:p
    for key, a in aa.items():
        b = ba[key]
        for art in (a,b):
            for name,h in art['hashes'].items():
                assert digest(Path(art['directory'])/name) == h
        if key[2] == 'vbcc':
            for name,h in a['hashes'].items():
                if name == 'code.lst':
                    old = (Path(b['directory'])/name).read_bytes()
                    assert old.replace(f'Source: "{b["directory"]}/code.asm"'.encode(), f'Source: "{a["directory"]}/code.asm"'.encode()) == (Path(a['directory'])/name).read_bytes()
                else:
                    assert b['hashes'][name] == h
            continue
        old_image, new_image = [json.loads(Path(v['image']).read_text()) for v in (b,a)]
        expected_artifact = copy.deepcopy(b)
        if key == changed:
            ranges = [(v['address'],v['address']+len(v['bytes'])) for v in old_image['segments'] if v['executable']]
            expected, mapping, removed, remap = instructions(all_instructions(old_image),s,ranges)
            assert all_instructions(new_image) == expected
            assert new_image == image(old_image,expected,remap,s)
            counted = [(pc,code) for pc,code in expected if any(remap(lo) <= pc < remap(hi) for lo,hi in b['code_ranges'])]
            assert listing(Path(a['directory'])/'code.asm') == counted
            expected_artifact['entry'] = remap(b['entry'])
            expected_artifact['code_ranges'] = [[remap(lo),remap(hi)] for lo,hi in b['code_ranges']]
            expected_artifact['guard_ranges'] = [[remap(lo),remap(hi)] for lo,hi in b['guard_ranges']]
            expected_artifact['code_bytes'] -= 8
            expected_artifact['routines'] = [routine(r,remap,s) for r in b['routines']]
            for arg in expected_artifact['arguments']:
                arg['body_displacement'] -= 4
        else:
            assert a['hashes'] == b['hashes'] and new_image == old_image, key
        for k in ('directory','commands','image','hashes'):
            expected_artifact[k] = a[k]
        assert a == expected_artifact, key
    forecasts = {f['vector']:f for f in frozen['forecasts']}
    for key, old in before.items():
        expected = old if key[2] == 'vbcc' else measurements(old,s if key[:3] == changed else None,forecasts,mapping,removed,remap)
        assert after[key] == expected, (key,{k:(expected.get(k),after[key].get(k)) for k in expected.keys()|after[key].keys() if expected.get(k) != after[key].get(k)})
        if key[2] == 'actionc':
            assert after[key]['correct']
    # Control proof identities follow code positions, never source-load identity.
    old_control = json.loads((before_dir/'debug.json').read_text())['control']
    new_control = json.loads((after_dir/'debug.json').read_text())['control']
    expected_control = copy.deepcopy(old_control)
    for record in expected_control:
        if (record['case'],record['mode'],'actionc') == changed:
            for proof in record['sites']:
                for field in ('pc','target'):
                    if field in proof:
                        proof[field] = remap(proof[field])
    assert new_control == expected_control
    failures = [list(k) for k,r in after.items() if not r['correct']]
    assert failures == [frozen['known_external_failure']]
    return dict(records=len(after), action_instruction_streams=28, complete_action_images=28,
                unchanged_action_builds=27, changed_builds=[list(changed)],
                results=[dict(vector=f['vector'],before=f['before'],after=f['forecast'],
                              selective_counters=f['expected_selective_counters']) for f in frozen['forecasts']],
                totals_per_incoming_i=frozen['corpus_totals_per_incoming_i'], external_failures=failures)


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('before',type=Path); p.add_argument('after',type=Path)
    p.add_argument('--baseline',type=Path,required=True); p.add_argument('--output',type=Path,required=True)
    a = p.parse_args()
    result = check(a.before,a.after,json.loads(a.baseline.read_text()))
    a.output.parent.mkdir(parents=True,exist_ok=True)
    a.output.write_text(json.dumps(result,indent=2)+'\n')
    print({k:v for k,v in result.items() if k != 'results'})
