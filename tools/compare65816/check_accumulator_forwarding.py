#!/usr/bin/env python3
"""Prove declared private LDA removal in final instruction streams and VM traffic."""
import argparse
import hashlib
import json
from pathlib import Path
import sys
sys.dont_write_bytecode = True
from delta import load, check_records, forwarded_word_load_counts, frame_contract
from check_empty_edges import listing


def check_listing(before, after, sites):
    positions = {pc:i for i,(pc,_) in enumerate(before)}
    removed = set()
    for pc in sites:
        assert pc in positions, ('missing reload',pc)
        i = positions[pc]
        assert i > 0 and i+1 < len(before)
        code = before[i][1]
        assert len(code)==2 and code[0]==0xa3 and 1<=code[1]<=254
        assert before[i-1][1]==bytes([0x83,code[1]]), ('missing retained producer store',pc)
        assert i not in removed, ('duplicate reload',pc)
        removed.add(i)
    kept = [ins for i,ins in enumerate(before) if i not in removed]
    assert len(kept)==len(after), ('instruction count',len(kept),len(after))
    addresses = {old[0]:new[0] for old,new in zip(kept,after)}
    for (old_pc,old),(new_pc,new) in zip(kept,after):
        if old[0] in (0x22,0x5c):
            target=int.from_bytes(old[1:],'little')
            assert target not in sites, ('independent entry into omitted reload',target)
            old=old[:1]+addresses.get(target,target).to_bytes(3,'little')
        assert old==new, ('unexpected instruction change',hex(old_pc),hex(new_pc),old.hex(),new.hex())
    return {pc:addresses[before[positions[pc]+1][0]] for pc in sites}


def check(before_dir,after_dir,counts,inventory,baseline):
    for field in ('qualification_record','measurements','snapshot_hashes'):
        for path,digest in baseline[field].items():
            assert hashlib.sha256(Path(path).read_bytes()).hexdigest()==digest,path
    old_manifest,old=load(before_dir);new_manifest,new=load(after_dir)
    predicted=forwarded_word_load_counts(counts)
    check_records(old,new,{},forwarded=predicted)
    assert old_manifest['cases']==new_manifest['cases']
    for tool in ('vbcc','vasm','vlink'):
        assert old_manifest['tools'][tool]['sha256']==new_manifest['tools'][tool]['sha256']
    original={(a['case'],a['mode'],a['compiler']):a for a in old_manifest['artifacts']}
    selected={(a['case'],a['mode']):a for a in inventory['builds']}
    assert len(selected)==len(inventory['builds'])==28
    rows=[]
    for a in new_manifest['artifacts']:
        key=a['case'],a['mode'],a['compiler'];b=original[key]
        for artifact in (a,b):
            for name,digest in artifact['hashes'].items():
                assert hashlib.sha256((Path(artifact['directory'])/name).read_bytes()).hexdigest()==digest
        if a['compiler']=='vbcc':
            for name in a['hashes']:
                if name=='code.lst':
                    old_text=(Path(b['directory'])/name).read_bytes();new_text=(Path(a['directory'])/name).read_bytes()
                    old_header=f'Source: "{b["directory"]}/code.asm"'.encode();new_header=f'Source: "{a["directory"]}/code.asm"'.encode()
                    assert old_text.count(old_header)==new_text.count(new_header)==1
                    assert old_text.replace(old_header,new_header)==new_text,key
                else:assert a['hashes'][name]==b['hashes'][name],(key,name)
            continue
        assert [frame_contract(r) for r in a['routines']]==[frame_contract(r) for r in b['routines']]
        sites=selected[key[:2]]['reload_pcs']
        remap=check_listing(listing(Path(b['directory'])/'code.asm'),listing(Path(a['directory'])/'code.asm'),sites)
        assert b['code_bytes']-a['code_bytes']==2*len(sites),key
        reached={int(pc) for k,r in new.items() if k[:3]==key for pc in r['forwarded_word_load_sites']}
        assert reached==set(remap.values()),(key,'actual forwarded sites',reached,remap)
        if not sites:assert a['hashes']==b['hashes'],key
        rows.append(dict(case=key[0],mode=key[1],reloads=len(sites),pc_map=remap))
    for f in baseline['forecasts']:
        key=f['case'],f['mode'],'actionc',f['vector']
        for field,value in f['before'].items():assert old[key][field]==value,(key,field)
        for field,value in f['forecast'].items():assert new[key][field]==value,(key,field)
        for field in f['unchanged_fields']:assert new[key][field]==old[key][field],(key,field)
        assert new[key]['forwarded_word_loads']==f['executed_reload_count']
    return dict(records=len(new),action_builds=len(rows),changed_builds=sum(r['reloads']>0 for r in rows),
                selected_static_sites=sum(r['reloads'] for r in rows),selected_vector_records=len(predicted),
                executed_forwarded_loads_per_incoming_i_state=sum(predicted.values()),
                only_declared_loads_removed=True,forecasts_exact=True,unchanged_storage_and_guards=True,sites=rows)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('before',type=Path);p.add_argument('after',type=Path)
    for name in ('counts','sites','baseline','output'):p.add_argument('--'+name,type=Path,required=True)
    a=p.parse_args()
    result=check(a.before,a.after,json.loads(a.counts.read_text()),json.loads(a.sites.read_text()),json.loads(a.baseline.read_text()))
    a.output.write_text(json.dumps(result,indent=2)+'\n')
    print(f'Checked {result["records"]} records, {result["action_builds"]} streams, {result["selected_static_sites"]} reload sites')
if __name__=='__main__':main()
