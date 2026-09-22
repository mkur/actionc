#!/usr/bin/env python3
"""Check frozen acyclic-copy savings and every surviving machine instruction."""
import argparse
import hashlib
import json
from pathlib import Path
import sys
sys.dont_write_bytecode = True
from check_empty_edges import listing
from delta import load, frame_contract
from inventory_copies import validate_edge


def digest(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def transform(before, after, edges):
    positions = {pc:i for i,(pc,_) in enumerate(before)}
    chunks, removed = {}, set()
    for e in edges:
        pcs = validate_edge(e,dict(before))
        assert pcs == e['copy_pcs']
        n = len(e['moves']); start = positions[pcs[0]]
        assert e['analysis']['graph'] in ('independent','ordered_acyclic')
        assert len(pcs) == 4*n and [positions[p] for p in pcs] == list(range(start,start+4*n))
        assert start not in chunks
        # Captured values and destination stores survive, interleaved in order.
        chunks[start] = (4*n,[start+offset for i in range(n) for offset in (2*i,2*n+2*i+1)])
        removed.update(start+offset for i in range(n) for offset in (2*i+1,2*n+2*i))
    order=[];i=0
    while i<len(before):
        if i in chunks:
            size, retained=chunks[i];order.extend(retained);i+=size
        else:order.append(i);i+=1
    assert len(order)==len(after)
    mapping={before[i][0]:new[0] for i,new in zip(order,after)}
    for i,(pc,new) in zip(order,after):
        old_pc,old=before[i]
        if old[0] in (0x22,0x5c,0xaf,0x8f):
            target=int.from_bytes(old[1:],'little')
            assert target not in {before[i][0] for i in removed}
            expected=old[:1]+mapping.get(target,target).to_bytes(3,'little')
        elif old[0] in (0x10,0x30,0x90,0xb0,0xd0,0xf0):
            target=old_pc+2+int.from_bytes(old[1:],'little',signed=True)
            delta=mapping[target]-pc-2
            assert -128<=delta<=127
            expected=old[:1]+bytes([delta&255])
        elif old[0]==0x62:
            resume=old_pc+3+int.from_bytes(old[1:],'little',signed=True)+1
            expected=old[:1]+(mapping[resume]-1-pc-3).to_bytes(2,'little',signed=True)
        else:expected=old
        assert new==expected,('unexpected instruction',hex(old_pc),hex(pc),old.hex(),new.hex(),expected.hex())
    return mapping,{before[i][0] for i in removed}


def check(before_dir,after_dir,frozen):
    assert digest('docs/benchmarks/65816-copy-inventory/inventory.json')==frozen['inventory_sha256']
    for name,h in frozen['comparison_hashes'].items():assert digest(before_dir/name)==h
    bm,before=load(before_dir);am,after=load(after_dir)
    assert before.keys()==after.keys() and bm['cases']==am['cases']
    for t in ('vbcc','vasm','vlink'):assert bm['tools'][t]['sha256']==am['tools'][t]['sha256']
    artifacts=lambda m:{(a['case'],a['mode'],a['compiler']):a for a in m['artifacts']}
    ba,aa=artifacts(bm),artifacts(am)
    for a in [*ba.values(),*aa.values()]:
        for name,h in a['hashes'].items():assert digest(Path(a['directory'])/name)==h
    maps,removed={},{}
    for key,a in aa.items():
        b=ba[key]
        if key[2]=='vbcc':
            for name,h in a['hashes'].items():
                if name=='code.lst':
                    old=(Path(b['directory'])/name).read_bytes();new=(Path(a['directory'])/name).read_bytes()
                    assert old.replace(f'Source: "{b["directory"]}/code.asm"'.encode(),f'Source: "{a["directory"]}/code.asm"'.encode())==new
                else:assert b['hashes'][name]==h
            continue
        edges=[e for e in frozen['expected_sites'] if (e['case'],e['mode'])==key[:2]]
        maps[key],removed[key]=transform(listing(Path(b['directory'])/'code.asm'),listing(Path(a['directory'])/'code.asm'),edges)
        assert [frame_contract(r) for r in b['routines']]==[frame_contract(r) for r in a['routines']]
        if not edges:assert b['hashes']==a['hashes'],('unaffected image changed',key)
    forecasts={(r['case'],r['mode'],'actionc',r['vector']):r for r in frozen['forecasts']}
    rows=[]
    for key,old in before.items():
        new=after[key]
        if key[2]=='vbcc':assert old==new;continue
        assert new['correct']
        extra={'acyclic_word_edges','acyclic_edge_words','acyclic_word_edge_sites'}
        assert set(new)==set(old)|extra
        f=forecasts.get(key,{})
        mapping=maps[key[:3]]
        counts={}
        for e in frozen['expected_sites']:
            if (e['case'],e['mode'])==key[:2]:
                count=old['instruction_sites'].get(str(e['copy_pcs'][0]),0)
                if count:counts[str(mapping[e['copy_pcs'][0]])]=count
        assert new['acyclic_word_edge_sites']==counts
        assert new['acyclic_word_edges']==sum(counts.values())==f.get('executions',0)
        assert new['acyclic_edge_words']==f.get('words',0)*f.get('executions',0)
        for field,value in old.items():
            if field in ('instruction_sites','fused_branch_sites','word_edge_sites','direct_word_edge_sites','forwarded_word_load_sites'):
                expected={str(mapping[int(pc)]):n for pc,n in value.items()
                          if not(field=='instruction_sites' and int(pc) in removed[key[:3]])}
                assert new[field]==expected,(key,field)
            elif field in ('code_bytes','instructions','cycles','stack_reads','stack_writes'):
                name='bytes' if field=='code_bytes' else field
                assert value-new[field]==f.get(name,0),(key,field,value,new[field],f)
            else:assert new[field]==value,(key,field)
        rows.append(dict(case=key[0],mode=key[1],vector=key[3],cycles_saved=f.get('cycles',0),stack_reads_saved=f.get('stack_reads',0),stack_writes_saved=f.get('stack_writes',0)))
    return dict(records=len(after),instruction_streams=len(maps),selected_sites=len(frozen['expected_sites']),
                executed_edges_per_incoming_i=sum(r['executions'] for r in frozen['forecasts']),
                cycles_saved_per_incoming_i=sum(r['cycles'] for r in frozen['forecasts']),deltas=rows,
                external_failures=[list(k) for k,r in after.items() if not r['correct']])


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('before',type=Path);p.add_argument('after',type=Path)
    p.add_argument('--baseline',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();result=check(a.before,a.after,json.loads(a.baseline.read_text()))
    a.output.parent.mkdir(parents=True,exist_ok=True);a.output.write_text(json.dumps(result,indent=2)+'\n')
    print({k:v for k,v in result.items() if k!='deltas'})
