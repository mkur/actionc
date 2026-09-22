#!/usr/bin/env python3
"""Freeze the bounded INX transform from typed post-X facts and measured bytes."""
import argparse
import copy
import json
from pathlib import Path
import sys
sys.dont_write_bytecode = True
from loop_x import candidate, canonical, ROOT
from inventory_registers import verify_snapshot
from report import digest
from delta import load
from check_selective_staging import all_instructions, relocate


def select(r, ins):
    t = candidate(r)
    if t['rejections']:
        return t
    lo, hi = t['update_range']
    if ins[lo] == b'\xc2\x20': lo += 2
    expected = {lo: b'\x8a', lo+1: b'\x18', lo+2: b'\x69\x01\x00'}
    for pc, code in expected.items(): assert ins[pc] == code
    q = next(h for h in r['temp_homes'] if h['id'] == t['update'])['home']
    assert q['kind'] == 'dp' and q['width'] == 2 and q != t['home']
    assert ins[lo+5] == bytes([0x85, q['offset']]) and hi == lo+7
    at = t['compare_range'][0]
    if ins[at] == b'\xc2\x20': at += 2
    assert ins[at] == b'\xe0' + t['threshold'].to_bytes(2, 'little')
    for e in t['incoming']:
        assert ins[e['transfer_pc']-1] == b'\xaa'
    # The closed scalar whitelist has no flag-valued MIR operand. ADD/SUB
    # establish their carry input; comparisons establish C/Z before branches.
    t.update(update_pc=lo, removed_pc=lo+2, update_home=q,
             before={str(pc):code.hex() for pc,code in expected.items()},
             flags='C/V are not MIR results; all admitted flag consumers define their inputs')
    return t


def transform(before, t):
    ins = all_instructions(before); old = dict(ins)
    for pc, code in t['before'].items(): assert old[int(pc)].hex() == code
    removed = t['removed_pc']; start = t['update_pc']
    replacements = {start: b'\xe8', start+1: b'\x8a'}
    executable = [(s['address'], s['address']+len(s['bytes'])) for s in before['segments'] if s['executable']]
    def remap(pc):
        assert not removed <= pc < removed+3, 'reference into removed ADC'
        return pc-3 if pc >= removed+3 and any(lo<=pc<=hi for lo,hi in executable) else pc
    expected = [(remap(pc), relocate(pc, replacements.get(pc,code), remap(pc), remap))
                for pc,code in ins if pc != removed]
    image = copy.deepcopy(before); image['entry'] = remap(image['entry'])
    for s in image['segments']:
        if not s['executable']: continue
        lo,hi=s['address'],s['address']+len(s['bytes']);s['address']=remap(lo)
        s['bytes']=list(b''.join(code for pc,code in expected if remap(lo)<=pc<remap(hi)))
        assert len(s['bytes'])==remap(hi)-remap(lo)
    for r in image['routines']:
        lo=r['address'];r['address']=remap(lo);r['size']=remap(lo+r['size'])-r['address']
    return image,expected,remap,{}


def measurement(old, t=None, remap=lambda p:p, unused=None):
    out=copy.deepcopy(old);out.update(x_increment_updates=0,x_increment_update_sites={})
    if t is None:return out
    at=t['update_pc']; n=old['instruction_sites'].get(str(at),0)
    assert old['instruction_sites'].get(str(at+1),0)==n
    assert old['instruction_sites'].get(str(at+2),0)==n
    assert old['x_forwarded_load_sites']==({str(at):n} if n else {})
    out['code_bytes']-=3;out['cycles']-=3*n;out['instructions']-=n
    for field,value in old.items():
        if field.endswith('_sites'):
            out[field]={str(remap(int(pc))):count for pc,count in value.items() if int(pc)!=t['removed_pc']}
    # INX changes X to q before TXA; that transfer is no longer a load of p.
    out['x_forwarded_loads']=0;out['x_forwarded_load_sites']={}
    out['x_increment_updates']=n
    if n:out['x_increment_update_sites']={str(remap(at)):n}
    return out


def freeze(directory, facts, snapshot):
    verify_snapshot(directory,snapshot)
    manifest,records=load(directory);builds=[]
    for artifact in manifest['artifacts']:
        for name,h in artifact['hashes'].items():assert digest(Path(artifact['directory'])/name)==h
    for f in facts['builds']:
        a,=[a for a in manifest['artifacts'] if (a['case'],a['mode'],a['compiler'])==(f['case'],f['mode'],'actionc')]
        image=json.loads(Path(a['image']).read_text());ins=dict(all_instructions(image))
        trials=[select(r,ins) for r in f['routines']];selected=[t for t in trials if not t['rejections']]
        assert len(selected)<=1
        t=selected[0] if selected else None
        expected,_,remap,_=transform(image,t) if t else (image,[],lambda p:p,{})
        projections=[dict(vector=r['vector'],forecast=measurement(r,t,remap)) for k,r in records.items() if k[:3]==(f['case'],f['mode'],'actionc')]
        builds.append(dict(case=f['case'],mode=f['mode'],trials=trials,selected=t,expected_image_sha256=canonical(expected),projections=projections))
    assert len(builds)==28 and sum(b['selected'] is not None for b in builds)==1
    return dict(schema=1,comparison_hashes={name:digest(directory/name) for name in ('manifest.json','debug.json','release.json')},builds=builds)


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('directory',type=Path);p.add_argument('--facts',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();result=freeze(a.directory,json.loads(a.facts.read_text()),ROOT/'docs/benchmarks/65816-loop-x/after');result['facts_sha256']=digest(a.facts)
    a.output.parent.mkdir(parents=True,exist_ok=True);a.output.write_text(json.dumps(result,indent=2)+'\n')
    print('Frozen 28 images and 132 records; one bounded INX candidate')
