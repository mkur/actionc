#!/usr/bin/env python3
"""Check a frozen coalescing transform against complete images and VM counts."""
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
    instructions = all_instructions(before)
    old = dict(instructions)
    removed=set(site['removed_pcs'])
    start=min(removed); end=max(removed)+2
    assert sorted(removed)==list(range(start,end,2))
    assert b''.join(code for at,code in instructions if start<=at<site['old_transfer_pc']).hex()==site['before_copy_bytes']
    patches={p['pc']:(bytes.fromhex(p['before']),bytes.fromhex(p['after'])) for p in site['operand_patches']}
    for at,(b,a) in patches.items(): assert old[at]==b and len(b)==len(a)==2
    assert all(len(old[p])==2 for p in removed)
    ranges=[(s['address'],s['address']+len(s['bytes'])) for s in before['segments'] if s['executable']]
    remap=lambda at:at-(end-start) if at>=end and any(lo<=at<=hi for lo,hi in ranges) else at
    def target(at):
        assert not start<=at<end, 'reference into removed edge copies'
        return remap(at)
    expected=[(remap(at),relocate(at,patches.get(at,(code,code))[1],remap(at),target)) for at,code in instructions if at not in removed]
    result=copy.deepcopy(before)
    result['entry']=target(result['entry'])
    for segment in result['segments']:
        if not segment['executable']:continue
        lo,hi=segment['address'],segment['address']+len(segment['bytes'])
        segment['address']=remap(lo)
        segment['bytes']=list(b''.join(code for at,code in expected if remap(lo)<=at<remap(hi)))
        assert len(segment['bytes'])==remap(hi)-remap(lo)
    result['routines']=[routine(r,remap,site) for r in result['routines']]
    assert b''.join(code for at,code in expected if start<=at<remap(site['old_transfer_pc'])).hex()==site['after_copy_bytes']
    return result,expected,remap


def routine(r,remap,site):
    r=copy.deepcopy(r);at=r['address']
    r['address']=remap(at);r['size']=remap(at+r['size'])-r['address']
    for call in r['calls']:assert set(call)=={'outgoing','transfer_peak'}
    if r['id']==site['routine']:
        for change in site['home_changes']:
            temp=next(t for t in r['temporaries'] if t['id']==change['temp'])
            assert temp['size']==2 and temp['home']==dict(kind='stack',displacement=change['old_offset'])
            temp['home']['displacement']=change['new_offset']
    return r


def measurement(old,site,remap):
    out=copy.deepcopy(old)
    out.update(coalesced_word_copies=0,coalesced_word_copy_sites={})
    if site is None:return out
    removed=set(site['removed_pcs']);first=min(removed)
    counts=[old['instruction_sites'].get(str(p),0) for p in sorted(removed)]
    assert len(set(counts))==1
    n=counts[0]
    out['code_bytes']-=8;out['cycles']-=20*n;out['instructions']-=4*n
    out['stack_reads']-=4*n;out['stack_writes']-=4*n
    out['coalesced_word_copies']=2*n
    out['coalesced_word_copy_sites']={str(first):2*n} if n else {}
    for field,value in old.items():
        if field.endswith('_sites'):
            out[field]={str(remap(int(at))):n for at,n in value.items() if field!='instruction_sites' or int(at) not in removed}
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
            expected['code_bytes']-=8
            expected['routines']=[routine(r,remap,s) for r in b['routines']]
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
                changed_builds=[list(changed)],removed_static_bytes=8,coalesced_copies_per_incoming_i=12,
                saved_cycles_per_incoming_i=120,saved_stack_reads_per_incoming_i=24,saved_stack_writes_per_incoming_i=24,
                nonedge_stores_frames_abi_and_guards_unchanged=True,forecasts=frozen['forecasts'],external_failures=failures)


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('before',type=Path);p.add_argument('after',type=Path)
    p.add_argument('--baseline',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
    a=p.parse_args()
    result=check(a.before,a.after,json.loads(a.baseline.read_text()))
    a.output.parent.mkdir(parents=True,exist_ok=True)
    a.output.write_text(json.dumps(result,indent=2)+'\n')
    print({k:v for k,v in result.items() if k!='forecasts'})
