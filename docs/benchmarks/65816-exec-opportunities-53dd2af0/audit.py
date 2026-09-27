"""Offline analysis, run from the repository root after rebuilding probe.rs.

python3 -B docs/benchmarks/65816-exec-opportunities-53dd2af0/audit.py
Inputs/output: target/exec-ranking-53dd2af0/current.{image,inventory}.json
No compiler mutation or execution qualification.
"""
from pathlib import Path
import collections,csv,hashlib,json,re,sys
sys.dont_write_bytecode=True
ROOT=Path.cwd();OUT=ROOT/'target/exec-ranking-53dd2af0'
sys.path.insert(0,str(ROOT/'tools'));from disassemble65816 import disassemble,DP,STACK,BRANCH
sys.path.insert(0,str(ROOT/'tools/compare65816'));from build import image_guard_ranges
read=lambda p:json.loads(p.read_text())
save=lambda name,d:(OUT/name).write_text(json.dumps(d,indent=2)+'\n')
image=read(OUT/'current.image.json');inv=read(OUT/'current.inventory.json')
assert image==read(ROOT/'target/exec-current-audit/immediate-push.image.json')
for f in read(ROOT/'target/exec-current-audit/frozen-inputs.json'):
 assert hashlib.sha256((ROOT/'target/exec-current-audit/frozen-exec'/f['path']).read_bytes()).hexdigest()==f['snapshot_sha256']
ins={};texts={};modes={};instructions={};routines={r['id']:r for r in image['routines']};spans=[];byspan={};cats=collections.Counter();ops=collections.defaultdict(collections.Counter);families=collections.defaultdict(collections.Counter)
for r,m,seg in zip(image['routines'],inv['routines'],[s for s in image['segments'] if s['executable']]):
 assert r['id']==m['id'] and r['address']==seg['address'];base=r['address'];guardranges=image_guard_ranges(image,seg);guards={pc for a,b in guardranges for pc in range(a,b)};m8=False;seq=[]
 for line in disassemble(dict(image,segments=[seg])).splitlines():
  match=re.fullmatch(r'([0-9A-F]{6})  ((?:[0-9A-F]{2} )*[0-9A-F]{2})\s+(.+)',line);assert match,line
  pc=int(match[1],16);code=bytes.fromhex(match[2]);op=code[0];ins[pc]=code;texts[pc]=match[3];modes[pc]=m8;seq.append(pc)
  if op in (0xc2,0xe2) and code[1]&32:m8=op==0xe2
  key=('guards' if pc in guards else 'stack_relative' if op in STACK else 'direct_page' if op in DP else 'mode_changes' if op in (0xc2,0xe2) else 'JSL' if op==0x22 else 'JML' if op==0x5c else 'branches' if op in BRANCH or op==0x82 else 'indirect_memory' if op in (0xa7,0x87,0xb7,0x97) else 'absolute_long_memory' if op in (0xaf,0x8f) else 'other')
  cats[key]+=len(code)
 instructions[r['id']]=seq
 covered=set()
 for s in m['spans']:
  start,end=base+s['start'],base+s['end'];assert not covered.intersection(range(start,end));covered.update(range(start,end));g=len(guards.intersection(range(start,end)));s=s|dict(routine_id=r['id'],routine=r['name'],pc=start,bytes=end-start,body_bytes=end-start-g,guard_bytes=g)
  spans.append(s);byspan[r['id'],s['block'],s['index']]=s
  ops[s['kind']].update(sites=1,bytes=s['bytes'],body_bytes=s['body_bytes'])
  k=s['kind'].split('/')[0];families[k].update(sites=1,body_bytes=s['body_bytes'])
 families['Prologue/helpers/unspanned'].update(sites=1,body_bytes=len(set(range(base,base+r['size']))-guards-covered))
assert sum(cats.values())==331914
assert sum(v['body_bytes'] for v in families.values())==331914-cats['guards']
save('spans.json',spans);save('categories.json',dict(cats));save('operations.json',dict(sorted(ops.items(),key=lambda kv:-kv[1]['body_bytes'])));save('families.json',dict(sorted(families.items(),key=lambda kv:-kv[1]['body_bytes'])))
print('CATEGORIES',dict(cats));print('FAMILIES',dict(sorted(families.items(),key=lambda kv:-kv[1]['body_bytes'])))
def select(name,selected):
 result=dict(sites=len(selected),body_bytes=sum(s['body_bytes'] for s in selected));print(name,result);save(name+'.json',selected);return result
select('pointer-private-loads',[s for s in spans if s['kind'] in ['Load/3/Parameter','Load/3/AutomaticFrame']])
select('pointer-indirect-accesses',[s for s in spans if s['kind'].split('/')[0] in ['Load','Store'] and '/LongIndirect' in s['kind']])
select('indexed-accesses',[s for s in spans if s['kind'].split('/')[0] in ['Load','Store'] and '/LongIndexed' in s['kind']])
select('integer-casts',[s for s in spans if s['kind'].startswith('Cast/') and s['kind'].endswith('/Integer') and not s['kind'].startswith('Cast/3->3')])

def temp_def(op):
    match=re.search(r'^\w+ \{ dest: TempId\((\d+)\)',op)
    if not match:match=re.search(r'^Call.*result: Some\(\(TempId\((\d+)\)',op)
    return int(match[1]) if match else None
def uses(op):
    ids=[int(i) for i in re.findall(r'TempId\((\d+)\)',op)];d=temp_def(op)
    if d is not None:ids.remove(d)
    return ids
def pcs(s):return [p for p in instructions[s['routine_id']] if s['pc']<=p<s['pc']+s['bytes']]
def split_items(text):
    level=0;start=0;items=[]
    for i,c in enumerate(text):
        if c in '([{':level+=1
        elif c in ')]}':level-=1
        elif c==',' and level==0:items.append(text[start:i].strip());start=i+1
    if text[start:].strip():items.append(text[start:].strip())
    return items

cohorts=collections.defaultdict(list)
for m in inv['routines']:
    r=routines[m['id']];params={p['id']:p for p in m['parameters']};objects={o['id']:o for o in m['objects']};homes={t['id']:t for t in r['temporaries']};alluses=collections.defaultdict(list)
    allops=[o for b in m['blocks'] for o in b['ops']]
    for b in m['blocks']:
        for i,op in enumerate(b['ops']+[b['terminator']]):
            for t in uses(op):alluses[t].append((b['id'],i,op))
    for b in m['blocks']:
        for i,op in enumerate(b['ops']):
            s=byspan[m['id'],b['id'],i];d=temp_def(op);code=pcs(s)
            if s['kind'].startswith('Binary/1/') and s['kind'].split('/')[-1] in ['Add','Sub','And','Or','Xor']:
                staging=[j for j in range(1,len(code)) if ins[code[j]]==bytes([0x85,0x10])]
                # Discover exact RIGHT slot from the immediately following arithmetic use.
                match=[]
                for j in range(len(code)-3):
                    a,c,e,f=[ins[p] for p in code[j:j+4]]
                    if a[0] in [0xa3,0xa5,0xa9] and len(a)==2 and c[0]==0x85 and e[0] in [0xa3,0xa5,0xa9] and len(e)==2 and f[0] in [0x65,0xe5,0x25,0x05,0x45] and f[1]==c[1]:match.append(j)
                if len(match)==1:cohorts['byte_alu'].append(s|dict(saved=4))
            if s['kind'].startswith(('Load/1/Parameter','Load/2/Parameter','Load/3/Parameter','Load/4/Parameter','Load/1/AutomaticFrame','Load/2/AutomaticFrame','Load/3/AutomaticFrame','Load/4/AutomaticFrame')):
                w=int(s['kind'].split('/')[1]);kind=s['kind'].split('/')[2];id=int(re.search(r'base: '+kind+r'\((?:ParamId|Mir65816FrameObjectId)\((\d+)\)\)',op)[1]);ref='base: '+kind+'('+('ParamId' if kind=='Parameter' else 'Mir65816FrameObjectId')+f'({id}))'
                relevant=[o for o in allops if ref in o]
                unsafe=any(o.startswith(('AddressOf','Copy')) or 'volatile: true' in o or 'displacement: ByteOffset(0), index: None' not in o or f'width: ByteSize({w})' not in o for o in relevant)
                if kind=='Parameter':
                    p=params[id];unsafe |= p['frame_object'] is not None or any(o.startswith('Store') for o in relevant) or f'size: ByteSize({w})' not in p['incoming']
                    off=int(re.search(r'offset: ByteOffset\((\d+)\)',p['incoming'])[1]);source=next(a['body_displacement'] for a in r['arguments'] if a['offset']==off)
                else:
                    obj=objects[id];unsafe |= obj['addressable'] or obj['size']!=w or not obj['owner'].startswith('Local(')
                    source=next(o['displacement'] for o in r['objects'] if o['id']==id)
                if unsafe or homes[d]['home']['kind']!='stack' or not alluses[d]:continue
                # Count only actual, surviving full load/store captures. Existing zero spans and A-forwarded halves are excluded.
                transfer=[]
                for j in range(len(code)-1):
                    a,c=ins[code[j]],ins[code[j+1]]
                    if a[0]==0xa3 and c[0]==0x83 and a[1] in range(source,source+w) and c[1] in range(homes[d]['home']['displacement'],homes[d]['home']['displacement']+w):transfer.extend([code[j],code[j+1]])
                saved=4 if w in (1,2) else 8
                if len(set(transfer))*2!=saved:continue
                refs=alluses[d]
                if any(o.startswith('Cast') for _,_,o in refs):continue
                if w==3 and kind=='Parameter':
                    delta=max([int(re.search(r'outgoing_bytes: ByteSize\((\d+)\)',o)[1]) for _,_,o in refs if o.startswith('Call')]+[0])
                    if source+w-1+delta<=255:cohorts['whole_parameter_ceiling'].append(s|dict(saved=saved))
                if any(bid!=b['id'] for bid,_,_ in refs):continue
                last=max(at for _,at,_ in refs)
                window=b['ops'][i+1:min(last+1,len(b['ops']))]
                barriers=[i+1+j for j,o in enumerate(window) if o.startswith(('Call','Store','Copy')) or 'volatile: true' in o]
                if barriers and barriers!=[last]:continue
                final=(b['ops']+[b['terminator']])[last]
                if final.startswith('Call'):
                    if not final.startswith('Call { target: Direct'):continue
                    delta=int(re.search(r'outgoing_bytes: ByteSize\((\d+)\)',final)[1])
                    if source+w-1+delta>255:continue
                if final.startswith('Store') and (ref in final or 'volatile: true' in final):continue
                if any(not o.startswith(('Load','Store','Compare','Binary','PointerOffset','AddressOf','Call','Return')) for _,_,o in refs):continue
                if w==3:
                    if not barriers:continue
                    # Match current supported pointer consumers before the one final barrier.
                    bad=False
                    for _,at,o in refs:
                        if at==last:continue
                        if o.startswith(('Load','AddressOf')):
                            bad |= f'base: Indirect(Temp(TempId({d}), ByteSize(3)))' not in o or int(re.search(r'displacement: ByteOffset\((\d+)\)',o)[1])>65535
                        elif o.startswith('Compare'):bad |= 'width: ByteSize(3)' not in o
                        elif o.startswith('Binary'):bad |= 'width: ByteSize(3)' not in o or not re.search(r'operation: (Add|Sub),',o)
                        elif o.startswith('PointerOffset'):bad |= f'base: Temp(TempId({d}), ByteSize(3))' not in o
                        else:bad=True
                    if bad:continue
                    cohorts['pointer_terminal_'+final.split(' ',1)[0].lower()].append(s|dict(saved=saved,source=kind,final_index=last,source_displacement=source))
                else:
                    # First proposed scalar slice: single adjacent consumer, no address/index uses, no casts.
                    if len(refs)!=1 or last!=i+1 or 'base: Indirect' in final and f'TempId({d})' in final.split('value:')[0] or 'index: Some' in final:continue
                    if not final.startswith(('Compare','Binary','Call','Store')):continue
                    if final.startswith('Store') and f'value: Temp(TempId({d}), ByteSize({w}))' not in final:continue
                    if final.startswith(('Compare','Binary')) and f'width: ByteSize({w})' not in final:continue
                    consumer_span=byspan[m['id'],b['id'],last]
                    capture=homes[d]['home']['displacement']
                    reads_capture=any(ins[pc][0] in STACK and ins[pc][0]!=0x83 and capture<=ins[pc][1]<capture+w for pc in pcs(consumer_span))
                    # Retain the original load when existing A forwarding already removed the consumer reload.
                    saving=saved if reads_capture else saved//2
                    cohorts['scalar_adjacent'].append(s|dict(saved=saving,width=w,source=kind,final_index=last,consumer=final.split(' ',1)[0],retained_source_load=not reads_capture))

for name,rows in cohorts.items():
    save(name+'.json',rows)
    print(name,len(rows),'footprint',sum(s['body_bytes'] for s in rows),'modeled',sum(s['saved'] for s in rows),collections.Counter((s.get('width'),s.get('source'),s.get('consumer')) for s in rows))
save('candidate-totals.json',{name:dict(sites=len(rows),footprint=sum(s['body_bytes'] for s in rows),saved=sum(s['saved'] for s in rows)) for name,rows in cohorts.items()})
from functools import lru_cache

indexed=[];edges=[]
for m in inv['routines']:
    r=routines[m['id']];homes={t['id']:t for t in r['temporaries']};types={t['id']:t['type'] for t in m['temps']}
    for s in [s for s in spans if s['routine_id']==r['id']]:
        op=s['description'];code=pcs(s)
        if s['kind']=='AddressOf/3' and 'index: Some' in op:
            base=re.search(r'base: Indirect\(Temp\(TempId\((\d+)\), ByteSize\(3\)\)\)',op)
            idx=re.search(r'index: Some\(Mir65816Index \{ value: (.*?), stride: ByteSize\((\d+)\)',op)
            disp=int(re.search(r'displacement: ByteOffset\((\d+)\)',op)[1]);stride=int(idx[2]);dest=temp_def(op)
            if not base or homes[int(base[1])]['home']['kind']!='stack' or homes[dest]['home']['kind']!='stack':continue
            source=homes[int(base[1])]['home']['displacement'];destination=homes[dest]['home']['displacement']
            if abs(source-destination)<3:continue
            ix=re.fullmatch(r'Temp\(TempId\((\d+)\), ByteSize\(1\)\)',idx[1])
            if ix:
                id=int(ix[1]);ty=types[id]
                if 'signed: false' not in ty or homes[id]['home']['kind']!='stack' or 255*stride+disp>65535:continue
                # Exact A8 index read, A16 zero extension, bounded scale; full 24-bit add keeps bank carry.
                scale=stride.bit_length()-1
                if stride.bit_count()>1:scale+=2+3*(stride.bit_count()-1)
                replacement=9+scale+(4 if disp else 0)+13
                label='bounded BYTE index'
            elif re.fullmatch(r'U(?:8|16|24|32)\(\d+\)',idx[1]):
                off=int(re.search(r'\((\d+)\)',idx[1])[1])*stride+disp
                if off>65535:continue
                # Two overlapping private words for zero; native pointer add otherwise. Include explicit entry mode request.
                replacement=12 if off==0 else 18
                label='constant index'
            else:continue
            if replacement<s['body_bytes']:indexed.append(s|dict(saved=s['body_bytes']-replacement,replacement=replacement,subset=label))
        if s['kind']!='Terminator/Goto' or 'args: []' in op:continue
        target=int(re.search(r'target: BlockId\((\d+)\)',op)[1]);block=next(b for b in m['blocks'] if b['id']==target)
        widths=[w for _,w in block['params']];args=split_items(op.split('args: [')[1].split(']')[0])
        if not code or len(widths)!=len(args):continue
        # Only untouched all-byte two-phase fallbacks, excluding current word/pointer scheduling.
        payload=[p for p in code if ins[p][0] not in (0xc2,0xe2,0x80,0x82,0x5c)]
        if len(payload)!=4*sum(widths):continue
        if any(ins[payload[j]][0] not in (0xa3,0xa5,0xa9) or ins[payload[j+1]][0] not in (0x83,0x85) or not modes[payload[j]] or len(ins[payload[j]])!=2 for j in range(0,len(payload),2)):continue
        if any(not (a.startswith(('Temp(','U8(','U16(','U24(','U32(','Null('))) for a in args):continue
        # Every source is saved before any destination write; retain full staging and stack allocation.
        groups=[(w,not a.startswith('Temp(')) for w,a in zip(widths,args)]+[(w,False) for w in widths]
        @lru_cache(None)
        def best(i,mode):
            if i==len(groups):return 0 if mode==2 else 2
            w,constant=groups[i];options=[]
            layouts=[(1,)*w]
            if w>=2:layouts.append((2,)*(w//2)+(1,)*(w%2))
            if w==3:layouts.append((2,2))
            for layout in layouts:
                cost=0;state=mode
                for piece in layout:
                    cost+=(0 if state==piece else 2)+(5 if piece==2 and constant else 4);state=piece
                options.append(cost+best(i+1,state))
            return min(options)
        # Unknown entry costs no less than the actual instruction stream's first request.
        initial=1 if modes[code[0]] else 2
        prefix=0
        if ins[code[0]][0]==0xc2:
            # Retain the existing checked A16 boundary before the fallback.
            prefix=2;initial=2
        elif ins[code[0]][0]==0xe2:initial=0
        jumps=sum(len(ins[p]) for p in code if ins[p][0] in (0x80,0x82,0x5c))
        replacement=prefix+best(0,initial)+jumps
        if replacement<s['body_bytes']:edges.append(s|dict(saved=s['body_bytes']-replacement,replacement=replacement,widths=widths))

for name,rows in [('indexed_address_bounded',indexed),('native_edge_fallback',edges)]:
    save(name+'.json',rows);print(name,len(rows),'footprint',sum(s['body_bytes'] for s in rows),'modeled',sum(s['saved'] for s in rows),collections.Counter(s.get('subset',str(s.get('widths'))) for s in rows))

# Cross-cohort identity and private-home validation for the terminal-pointer model.
terminal=cohorts['pointer_terminal_store']+cohorts['pointer_terminal_call']
mir={r['id']:r for r in inv['routines']}
keys=lambda rows:{(s['routine_id'],s['block'],s['index']) for s in rows}
assert len(keys(terminal))==len(terminal)
assert keys(terminal).isdisjoint(keys(cohorts['scalar_adjacent']))
for s in terminal:
    m=mir[s['routine_id']];r=routines[s['routine_id']]
    source=s['source_displacement']
    assert 1<=source<=253
    assert all(t['home']['kind']!='stack' or source+3<=t['home']['displacement'] or t['home']['displacement']+t['size']<=source for t in r['temporaries'])
    b=next(b for b in m['blocks'] if b['id']==s['block'])
    op=b['ops'][s['final_index']];d=temp_def(s['description']);ref=f'Temp(TempId({d}), ByteSize(3))'
    if op.startswith('Store'):
        assert f'base: Indirect({ref})' in op or f'value: {ref}, width: ByteSize(3)' in op
        assert 'index: Some' not in op or ref not in op.split('index: Some')[1].split('mode:')[0]
    else:
        delta=int(re.search(r'outgoing_bytes: ByteSize\((\d+)\)',op)[1])
        assert source+2+delta<=255
print('Terminal pointer home/role checks passed:',len(terminal))
