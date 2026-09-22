#!/usr/bin/env python3
"""Typed read-only scalar DP admission and independently frozen image transform."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import sys
sys.dont_write_bytecode = True
from check_selective_staging import all_instructions, relocate
from inventory_movements import interference
from delta import load

MEMORY = {0xa3: (0xa5, 'read'), 0x83: (0x85, 'write'),
          0x63: (0x65, 'read'), 0xe3: (0xe5, 'read'), 0xc3: (0xc5, 'read')}

def digest(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()

def admission(r):
    reasons=set()
    if r['result'] not in ('word','void'): reasons.add('result')
    if any(p['width']!=2 for p in r['parameters']): reasons.add('parameters')
    if any(o['width']!=2 or o['addressable'] for o in r['fixed_objects']): reasons.add('objects')
    words=[]; booleans=set()
    for t in r['temp_homes']:
        ty=t['type']
        if ty['kind']=='integer' and ty['bits']==16 and ty['ordinary'] and ty['width']==2 and not ty['pointer']:
            words.append(t)
        elif ty['kind']=='bool' and ty['width']==1 and not ty['pointer']:
            booleans.add(t['id'])
        else: reasons.add('temporary_type')
        if t['home']['kind']!='stack': reasons.add('existing_dp_path')
    if not words: reasons.add('no_word_temps')
    defs=set()
    for b in r['blocks']:
        if any(w!=2 for w in b['parameter_widths']): reasons.add('edge_width')
        if b['terminator_kind']=='exit': reasons.add('exit')
        for op in b['ops']:
            if op.get('terminator'): continue
            if op['kind']=='compare': defs.add(op['definition'])
            if not op['native_word_form']: reasons.add('operation:'+op['kind'])
            if not op['safe_direct_memory']: reasons.add('memory')
            if op['effects']['barrier'] or op['effects']['calls']: reasons.add('effects')
            for v in op['values']:
                if not v['word_operand']: reasons.add('word_operand')
    if not booleans<=defs: reasons.add('boolean_definition')
    for e in r['edges']:
        if any(m['width']!=2 or not m['source']['word_operand'] for m in e['moves']): reasons.add('edge_operand')
    offsets=sorted({w['home']['offset'] for w in words})
    if len(offsets)>16: reasons.add('capacity')
    return sorted(reasons),words,offsets

def trial(r):
    reasons,words,offsets=admission(r)
    if reasons:return dict(routine=r['id'],rejections=reasons)
    mapping={o:32+2*i for i,o in enumerate(offsets)}
    homes={t['id']:t['home'] for t in r['temp_homes']}
    graph=[]
    for point,live in interference(r):
        ids=sorted(live)
        graph.append(dict(point=point,live=ids))
        for i,a in enumerate(ids):
            for b in ids[i+1:]:
                x,y=homes[a],homes[b]
                assert x['offset']+x['width']<=y['offset'] or y['offset']+y['width']<=x['offset'], (point,a,b)
    end=max([r['fixed_object_extent']]+[t['home']['offset']+t['home']['width']-1 for t in r['temp_homes'] if t not in words])
    cursor=end+1;staging=[]
    for old in r['staging_slots']:
        assert old['width']==2
        cursor=(cursor+1)&~1
        staging.append(dict(old=old['offset'],new=cursor,width=2));cursor+=2
    extent=(cursor//2)*2
    return dict(routine=r['id'],rejections=[],old_extent=r['frame_extent'],new_extent=extent,
                fixed_object_extent=r['fixed_object_extent'],staging=staging,
                classes=[dict(stack=o,dp=mapping[o],temps=sorted(t['id'] for t in words if t['home']['offset']==o)) for o in offsets],
                closed_live_points=graph)

def freeze_routine(r,t,instructions):
    """Only original instructions and typed homes determine patches/removals."""
    if t['rejections']:return t
    t=copy.deepcopy(t);at=r['address'];end=at+r['size'];ins={p:b for p,b in instructions if at<=p<end}
    old,new=t['old_extent'],t['new_extent'];saved=old-new
    assert saved>=0 and saved%2==0
    private={v['stack']:v['dp'] for v in t['classes']}
    stages={v['old']:v['new'] for v in t['staging']}
    incoming={p['offset']+old+4+i:p['offset']+new+4+i for p in r['parameters'] for i in range(p['width'])}
    patches={};accesses=[];removed=[]
    # The unchanged checked entry has fixed positions under current layout.
    for pc,opcode in [(at+0x15,0xe9),(at+0x26,0xa9)]:
        assert ins[pc]==bytes([opcode])+old.to_bytes(2,'little')
        if saved:patches[pc]=bytes([opcode])+new.to_bytes(2,'little')
    for pc,code in ins.items():
        if code==b'\x6b' and old:
            release={pc-8:b'\xa8',pc-7:b'\x3b',pc-6:b'\x18',pc-5:b'\x69'+old.to_bytes(2,'little'),pc-2:b'\x1b',pc-1:b'\x98'}
            assert all(ins[p]==b for p,b in release.items())
            if new==0: removed.extend(release)
            elif saved:patches[pc-5]=b'\x69'+new.to_bytes(2,'little')
        if code[0] not in MEMORY:continue
        off=code[1];op,kind=MEMORY[code[0]]
        if off in private:
            # Admission has no byte operations except comparison results; prove
            # those bytes and fixed objects/staging are disjoint from this home.
            others=[h['home'] for h in r['temp_homes'] if h['home']['width']!=2]
            others+=r['staging_slots']+[dict(offset=o['offset'],width=o['width']) for o in r['fixed_objects']]
            assert all(off+2<=s['offset'] or s['offset']+s['width']<=off for s in others)
            patches[pc]=bytes([op,private[off]])
            accesses.append(dict(pc=pc,access=kind,dp=private[off],width=2))
        elif off in stages:patches[pc]=bytes([code[0],stages[off]])
        elif off in incoming:patches[pc]=bytes([code[0],incoming[off]])
    t.update(patches=[dict(pc=p,before=ins[p].hex(),after=b.hex()) for p,b in sorted(patches.items())],
             removed_pcs=sorted(removed),accesses=accesses)
    return t

def transform(before,trials):
    ins=all_instructions(before);old=dict(ins);patches={};removed=set();changed={}
    for t in trials:
        if t['rejections']:continue
        changed[t['routine']]=t
        for p in t['patches']:
            assert old[p['pc']].hex()==p['before'] and p['pc'] not in patches
            patches[p['pc']]=bytes.fromhex(p['after'])
        assert not removed.intersection(t['removed_pcs']);removed.update(t['removed_pcs'])
    ranges=[(s['address'],s['address']+len(s['bytes'])) for s in before['segments'] if s['executable']]
    def remap(at):
        for lo,hi in ranges:
            if lo<=at<=hi:return at-sum(len(old[p]) for p in removed if p<at)
        return at
    def target(at):
        # Returns may be labels at the beginning of teardown; redirect to RTL.
        if at in removed:
            first=at
            while at in removed:at+=len(old[at])
            assert old[at]==b'\x6b' and old[first]==b'\xa8','entry into removed teardown interior'
        return remap(at)
    expected=[(remap(p),relocate(p,patches.get(p,b),remap(p),target)) for p,b in ins if p not in removed]
    out=copy.deepcopy(before);out['entry']=target(out['entry'])
    for s in out['segments']:
        if not s['executable']:continue
        lo,hi=s['address'],s['address']+len(s['bytes']);s['address']=remap(lo)
        s['bytes']=list(b''.join(b for p,b in expected if remap(lo)<=p<remap(hi)))
        assert len(s['bytes'])==remap(hi)-remap(lo)
    for r in out['routines']:
        at=r['address'];r['address']=remap(at);r['size']=remap(at+r['size'])-r['address']
        if r['id'] not in changed:continue
        t=changed[r['id']];saved=t['old_extent']-t['new_extent'];assert not r['calls']
        for field in ('fixed_frame','spill_bytes','local_stack_peak'):r[field]-=saved
        for a in r['arguments']:a['body_displacement']-=saved
        bytemp={i:c for c in t['classes'] for i in c['temps']}
        for temp in r['temporaries']:
            if temp['id'] in bytemp:
                c=bytemp[temp['id']];assert temp['home']==dict(kind='stack',displacement=c['stack'])
                temp['home']=dict(kind='direct_page',offset=c['dp'])
    return out,expected,remap,removed

def measurements(old,trials,remap,removed,old_image):
    out=copy.deepcopy(old);ins=dict(all_instructions(old_image));counts=old['instruction_sites']
    accesses=[s for t in trials if not t['rejections'] for s in t['accesses']]
    reads=sum(counts.get(str(s['pc']),0)*s['width'] for s in accesses if s['access']=='read')
    writes=sum(counts.get(str(s['pc']),0)*s['width'] for s in accesses if s['access']=='write')
    saved_cycles=sum(counts.get(str(p),0)*(3 if ins[p][0]==0x69 else 2) for p in removed)
    out['cycles']-= (reads+writes)//2+saved_cycles
    out['instructions']-=sum(counts.get(str(p),0) for p in removed)
    counted=[(r['address'],r['address']+r['size']) for r in old_image['routines'] if any(str(r['address'])==p for p in counts)]
    # Corpus records count worker/helper bodies, not unexecuted wrappers.
    out['code_bytes']-=sum(len(ins[p]) for p in removed if any(lo<=p<hi for lo,hi in counted))
    out['stack_reads']-=reads;out['stack_writes']-=writes
    out['dp_reads']+=reads;out['dp_writes']+=writes
    touched={s['dp']+i for s in accesses if counts.get(str(s['pc']),0) for i in range(s['width'])}
    out['dp_touched_offsets']=sorted(set(out['dp_touched_offsets'])|touched)
    for field,v in old.items():
        if field.endswith('_sites'):out[field]={str(remap(int(p))):n for p,n in v.items() if field!='instruction_sites' or int(p) not in removed}
    # All selected workers are leaves; direct_calls has the selected helper
    # below its unchanged caller's frame, argument reservation and JSL address.
    active=[t for t in trials if not t['rejections']]
    if active:
        assert len(active)==1
        out['peak_below_entry_s']-=active[0]['old_extent']-active[0]['new_extent']
    return out

def inventory(directory,facts):
    manifest,records=load(directory);builds=[]
    assert facts['lf_crlf_images_equal']
    for b in facts['builds']:
        artifact=next(a for a in manifest['artifacts'] if (a['case'],a['mode'],a['compiler'])==(b['case'],b['mode'],'actionc'))
        image=json.loads(Path(artifact['image']).read_text());instructions=all_instructions(image)
        assert len(b['routines'])==len(image['routines'])
        trials=[freeze_routine(r,trial(r),instructions) for r in b['routines']]
        expected,ins,remap,removed=transform(image,trials)
        projections=[]
        for (case,mode,compiler,vector),record in records.items():
            if (case,mode,compiler)==(b['case'],b['mode'],'actionc'):
                forecast=measurements(record,trials,remap,removed,image)
                projections.append(dict(vector=vector,forecast_sha256=hashlib.sha256(json.dumps(forecast,sort_keys=True).encode()).hexdigest(),
                    before={k:v for k,v in record.items() if not k.endswith('_sites')},
                    forecast={k:v for k,v in forecast.items() if not k.endswith('_sites')}))
        builds.append(dict(case=b['case'],mode=b['mode'],routines=trials,
                           expected_image_sha256=hashlib.sha256(json.dumps(expected,sort_keys=True).encode()).hexdigest(),
                           projections=projections))
    return dict(schema=1,compiler_unchanged=True,comparison_records=len(records),
                comparison_hashes={p:digest(directory/p) for p in ('manifest.json','debug.json','release.json')},builds=builds)

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('directory',type=Path);p.add_argument('--facts',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();result=inventory(a.directory,json.loads(a.facts.read_text()))
    a.output.parent.mkdir(parents=True,exist_ok=True);a.output.write_text(json.dumps(result,indent=2)+'\n')
    print('Inventoried',len(result['builds']),'builds;',sum(not r['rejections'] for b in result['builds'] for r in b['routines']),'eligible routines')
