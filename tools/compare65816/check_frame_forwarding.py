#!/usr/bin/env python3
"""Check one frozen frame reload deletion against complete images and VM counts."""
import argparse
import copy
import json
from pathlib import Path
import sys
sys.dont_write_bytecode = True
from check_selective_staging import all_instructions, relocate
from check_empty_edges import listing
from check_staging_reservations import digest
from delta import load


def transform(before, site):
    pc = site['reload_pc']
    instructions = all_instructions(before)
    old = dict(instructions)
    assert b''.join(old[at] for at in range(pc-4,pc+4,2)).hex() == site['before_bytes']
    assert old[pc] == bytes([0xa3,site['source_slot']])
    ranges = [(s['address'],s['address']+len(s['bytes'])) for s in before['segments'] if s['executable']]
    remap = lambda at: at-2 if at >= pc+2 and any(lo <= at <= hi for lo,hi in ranges) else at
    def target(at):
        assert at != pc, 'control transfer into removed reload'
        return remap(at)
    expected = [(remap(at),relocate(at,code,remap(at),target)) for at,code in instructions if at != pc]
    result = copy.deepcopy(before)
    result['entry'] = remap(result['entry'])
    for s in result['segments']:
        if not s['executable']:
            continue
        lo,hi = s['address'],s['address']+len(s['bytes'])
        s['address'] = remap(lo)
        s['bytes'] = list(b''.join(code for at,code in expected if remap(lo) <= at < remap(hi)))
        assert len(s['bytes']) == remap(hi)-remap(lo)
    result['routines'] = [routine(r,remap) for r in result['routines']]
    assert b''.join(code for at,code in expected if pc-4<=at<pc+2).hex() == site['after_bytes']
    return result,expected,remap


def routine(r,remap):
    r = copy.deepcopy(r)
    at = r['address']
    r['address'] = remap(at)
    r['size'] = remap(at+r['size'])-r['address']
    assert not r['calls'], 'frozen changed image has no calls'
    return r


def measurement(old,site,remap):
    out = copy.deepcopy(old)
    out.update(frame_forwarded_loads=0,frame_forwarded_load_sites={})
    if site is None:
        return out
    pc = site['reload_pc']
    n = old['instruction_sites'][str(pc)]
    assert n == 8
    out['code_bytes'] -= 2
    out['cycles'] -= 5*n
    out['instructions'] -= n
    out['stack_reads'] -= 2*n
    out['frame_forwarded_loads'] = n
    out['frame_forwarded_load_sites'] = {str(pc):n}
    for field,value in old.items():
        if field.endswith('_sites'):
            out[field] = {str(remap(int(at))):n for at,n in value.items() if field != 'instruction_sites' or int(at)!=pc}
    return out


def check(before_dir,after_dir,frozen):
    for name,h in frozen['comparison_hashes'].items():
        assert digest(before_dir/name)==h
    bm,before = load(before_dir); am,after = load(after_dir)
    assert before.keys()==after.keys() and bm['cases']==am['cases']
    for t in ('vbcc','vasm','vlink'):
        assert bm['tools'][t]['sha256']==am['tools'][t]['sha256']
    artifacts = lambda m:{(a['case'],a['mode'],a['compiler']):a for a in m['artifacts']}
    ba,aa = artifacts(bm),artifacts(am)
    assert ba.keys()==aa.keys()
    s = frozen['selected_site']; changed = (s['case'],s['mode'],'actionc')
    remap = lambda p:p
    for key,a in aa.items():
        b=ba[key]
        for art in (a,b):
            for name,h in art['hashes'].items():
                assert digest(Path(art['directory'])/name)==h
        if key[2]=='vbcc':
            for name,h in a['hashes'].items():
                if name=='code.lst':
                    old=(Path(b['directory'])/name).read_bytes()
                    assert old.replace(f'Source: "{b["directory"]}/code.asm"'.encode(),f'Source: "{a["directory"]}/code.asm"'.encode())==(Path(a['directory'])/name).read_bytes()
                else: assert b['hashes'][name]==h
            continue
        bi,ai = [json.loads(Path(v['image']).read_text()) for v in (b,a)]
        expected = copy.deepcopy(b)
        if key==changed:
            ei,ins,remap = transform(bi,s)
            assert ai==ei, 'complete image differs from frozen deletion'
            assert all_instructions(ai)==ins
            assert listing(Path(a['directory'])/'code.asm')==[(pc,code) for pc,code in ins if any(remap(lo)<=pc<remap(hi) for lo,hi in b['code_ranges'])]
            expected['entry']=remap(b['entry'])
            for field in ('code_ranges','guard_ranges'):
                expected[field]=[[remap(lo),remap(hi)] for lo,hi in b[field]]
            expected['code_bytes']-=2
            expected['routines']=[routine(r,remap) for r in b['routines']]
        else: assert a['hashes']==b['hashes'] and ai==bi,key
        for field in ('directory','commands','image','hashes'): expected[field]=a[field]
        assert a==expected,key
    for key,old in before.items():
        expected = old if key[2]=='vbcc' else measurement(old,s if key[:3]==changed else None,remap)
        assert after[key]==expected,(key,{k:(v,after[key].get(k)) for k,v in expected.items() if v!=after[key].get(k)})
        if key[2]=='actionc': assert after[key]['correct']
    for f in frozen['forecasts']:
        key=(*changed,f['vector'])
        for k,v in f['before'].items(): assert before[key][k]==v
        for k,v in f['forecast'].items(): assert after[key][k]==v
    old_control=json.loads((before_dir/'debug.json').read_text())['control']
    new_control=json.loads((after_dir/'debug.json').read_text())['control']
    control=copy.deepcopy(old_control)
    for record in control:
        if (record['case'],record['mode'],'actionc')==changed:
            for proof in record['sites']:
                for field in ('pc','target'):
                    if field in proof: proof[field]=remap(proof[field])
    assert new_control==control
    failures=[list(k) for k,r in after.items() if not r['correct']]
    assert failures==[frozen['known_external_failure']]
    return dict(records=len(after),complete_action_images=28,unchanged_action_builds=27,
                changed_builds=[list(changed)],removed_static_bytes=2,forwarded_loads_per_incoming_i=48,
                saved_cycles_per_incoming_i=240,saved_stack_reads_per_incoming_i=96,
                stores_frames_abi_and_guards_unchanged=True,forecasts=frozen['forecasts'],external_failures=failures)


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('before',type=Path);p.add_argument('after',type=Path)
    p.add_argument('--baseline',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
    a=p.parse_args()
    result=check(a.before,a.after,json.loads(a.baseline.read_text()))
    a.output.parent.mkdir(parents=True,exist_ok=True)
    a.output.write_text(json.dumps(result,indent=2)+'\n')
    print({k:v for k,v in result.items() if k!='forecasts'})
