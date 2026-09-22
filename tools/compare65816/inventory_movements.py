#!/usr/bin/env python3
"""Read-only inventory of copies, coalescing constraints and word reloads."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import sys
sys.dont_write_bytecode = True
from check_empty_edges import listing
from delta import load
from inventory_copies import classify, overlap

ROOT = Path(__file__).resolve().parents[2]


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def load_word(s):
    assert s['width'] == 2 and s['word_operand']
    return bytes([0xa3,s['offset']]) if s['kind'] == 'stack' else b'\xa9'+s['value'].to_bytes(2,'little')


def edge_code(e, slots, ins):
    """Independent physical-copy reconstruction; no compiler strategy is exported."""
    moves, end = e['moves'], e['transfer_pc']
    if e['fallthrough']:
        assert end == e['target_pc']
    else:
        assert ins[end] == b'\x5c'+e['target_pc'].to_bytes(3,'little')
    if not moves:
        return dict(form='empty',pcs=[],assignments=[],captures=[],reload=None)
    analysis = classify(moves)
    word = all(m['width']==2 and m['source']['word_operand'] and m['destination']['kind']=='stack' for m in moves)
    instructions, captures, assignments, reload = [], [], [], None
    for i,m in enumerate(moves):
        assert not any(overlap(m['destination'],v['destination']) for v in moves[:i])
    if word:
        if len(moves)==1 or (analysis['graph']!='cyclic' and not analysis['partial_overlaps']):
            order = [0] if len(moves)==1 else analysis['schedule']
            form = 'direct_word' if len(moves)==1 else 'acyclic_word'
        else:
            order = list(range(len(moves)))
            captures = analysis['in_order_staged_moves'] if not analysis['partial_overlaps'] else order
            form = 'selective_word' if not analysis['partial_overlaps'] else 'complete_word'
        stage = {i:slots[k] for k,i in enumerate(captures)}
        for k,i in enumerate(captures):
            assert all(not overlap(dict(stage[i],kind='stack'),dict(stage[j],kind='stack')) for j in captures[:k])
        for i in captures:
            assert stage[i]['width']>=2
            s = dict(stage[i],kind='stack')
            assert all(not overlap(s,m[k]) for m in moves for k in ('source','destination'))
            instructions += [load_word(moves[i]['source']),bytes([0x83,s['offset']])]
        for i in order:
            s = bytes([0xa3,stage[i]['offset']]) if i in stage else load_word(moves[i]['source'])
            assignments.append(dict(move=i,instruction=len(instructions)))
            instructions += [s,bytes([0x83,moves[i]['destination']['offset']])]
        if order[-1] != len(moves)-1:
            reload = len(instructions)
            instructions.append(bytes([0xa3,moves[-1]['destination']['offset']]))
    else:
        form='complete_byte';captures=list(range(len(moves)))
        for i,m in enumerate(moves):
            s=m['source'];assert slots[i]['width']>=m['width']
            scratch=dict(slots[i],kind='stack')
            assert all(not overlap(scratch,v[k]) for v in moves for k in ('source','destination'))
            assert all(not overlap(scratch,dict(v,kind='stack')) for v in slots[:i])
            for byte in range(m['width']):
                if s['kind']=='immediate':code=bytes([0xa9,(s['value']>>(8*byte))&255])
                else:
                    assert s['kind'] in ('stack','dp')
                    code=bytes([0xa3 if s['kind']=='stack' else 0xa5,s['offset']+byte])
                instructions += [code,bytes([0x83,slots[i]['offset']+byte])]
        for i,m in enumerate(moves):
            assignments.append(dict(move=i,instruction=len(instructions)))
            d=m['destination'];assert d['kind'] in ('stack','dp')
            for byte in range(m['width']):
                instructions += [bytes([0xa3,slots[i]['offset']+byte]),bytes([0x83 if d['kind']=='stack' else 0x85,d['offset']+byte])]
        instructions.append(b'\xc2\x20')
    at=end-sum(map(len,instructions));assert at>=e['block_pc']
    pcs=[]
    for code in instructions:
        assert ins.get(at)==code,('unexpected edge',hex(at),ins.get(at),code)
        pcs.append(at);at+=len(code)
    for a in assignments:
        a['load_pc']=pcs[a.pop('instruction')]
    return dict(form=form,pcs=pcs,assignments=assignments,captures=captures,
                reload=pcs[reload] if reload is not None else None)


def interference(r):
    """Independent closed-operation live points, including successor live-ins."""
    blocks={b['id']:b for b in r['blocks']}
    entries={i:set() for i in blocks}
    def exit_live(b):
        return set(b['terminator_uses']).union(*(entries[i] for i in b['successors']))
    while True:
        old={i:set(s) for i,s in entries.items()}
        for i,b in reversed(list(blocks.items())):
            live=exit_live(b)
            for op in reversed(b['ops']):
                if op.get('terminator'):continue
                live.discard(op['definition']);live.update(op['uses'])
            entries[i]=live-set(b['params'])
        if entries==old:break
    points=[]
    for i,b in blocks.items():
        live=exit_live(b)
        points.append((dict(block=i,point='exit'),set(live)))
        for op in reversed(b['ops']):
            if op.get('terminator'):continue
            live.update(op['uses'])
            if op['definition'] is not None:live.add(op['definition'])
            points.append((dict(block=i,operation=op['index'],point='closed_operation'),set(live)))
            live.discard(op['definition'])
        points.append((dict(block=i,point='parameters_and_live_ins'),live|set(b['params'])))
    return points


def coalescing(m, points, homes):
    s,d=m['source'],m['destination'];a,b=s.get('temp'),m['destination_temp']
    if s['kind']!=d['kind'] or s['kind']!='stack' or s['width']!=d['width'] or a is None:
        return dict(classification='not_private_same_width_temps')
    if s['offset']==d['offset']:
        return dict(classification='physical_self_copy', source_temp=a,destination_temp=b)
    witnesses=[point for point,live in points if a in live and b in live]
    attempts=[]
    for attempt in m['coalescing_attempts']:
        changed,onto=attempt['changed'],attempt['onto']
        blockers=sorted({other for _,live in points if changed in live for other in live
                         if other!=changed and overlap(homes[onto],homes[other])})
        error=attempt['verifier_error']
        assert bool(blockers)==(error=='overlapping live stack temporaries'), (m,blockers,error)
        assert blockers or error in (None,'invalid edge-copy slot count','invalid stack frame accounting','invalid or overlapping edge-copy staging slot')
        attempts.append(dict(**attempt,blocking_temps=blockers))
    return dict(classification='interfering_pair' if witnesses else 'compatible_pair',
                source_temp=a,destination_temp=b,witnesses=witnesses,directions=attempts,
                note='Compatibility is not a committed layout or additive saving. Recheck every edge, A/N/Z, scratch and frame after recoloring.')


def reloads(r, ins):
    """Conservative byte-identity simulation over final instructions within blocks.

    This is an observation tool, not the compiler tracker or an optimizer. Every
    label, call/helper, unknown operation, alias/volatile access and S change
    breaks the proof. CMP breaks N/Z while retaining A. Stores update byte homes.
    """
    owners={}
    for b in r['blocks']:
        for op in b['ops']:
            for pc in ins:
                if op['range'][0]<=pc<op['range'][1]:
                    assert pc not in owners
                    owners[pc]=dict(op,block=b['id'])
    edge_pcs={pc for e in r['edges'] for pc in e['encoding']['pcs']}
    labels=set(r['labels'])
    memory={};a=nz=origin=None;m8=False;epoch=0;cause='entry';rows=[];previous_owner=None
    def clear(reason):
        nonlocal a,nz,origin,epoch,cause
        a=nz=origin=None;memory.clear();epoch+=1;cause=reason
    def word(offset):
        return tuple(memory.get(offset+i,('memory',epoch,offset+i)) for i in (0,1))
    for pc,code in ins.items():
        op=code[0];owner=owners.get(pc)
        if pc in labels:clear('label_or_join')
        if owner is None or pc in edge_pcs:
            clear('outside_operation_or_edge')
        elif previous_owner != (owner['block'],owner['index']):
            if owner['kind'] in ('call','copy','pointer_offset','unary','cast','address_of'):
                clear('call_helper_or_unsupported_operation')
            if owner.get('volatile') or owner.get('address',{}).get('indexed') or owner.get('address',{}).get('kind')=='indirect':
                clear('volatile_or_alias_barrier')
        previous_owner=(owner['block'],owner['index']) if owner else None
        if op in (0xc2,0xe2):
            if code[1]&0x20:
                new_m8=op==0xe2
                if new_m8!=m8:clear('mode_change')
                m8=new_m8
            continue
        if op==0xa3 and not m8:
            slot=code[1];value=word(slot)
            # Source instructions include private temps, frame objects and
            # staging. Count all, but admit only typed operation-owned windows.
            equal=a is not None and a==value
            flags=equal and nz==a
            consumer=owner.get('consumer') if owner else None
            temp=(consumer and consumer.get('temp') is not None and consumer['kind']=='stack'
                  and consumer['width']==2 and consumer['offset']==slot)
            frame=(owner and owner['kind']=='load' and owner.get('width')==2
                   and owner.get('address',{}).get('kind') in ('frame','parameter')
                   and not owner['address']['indexed'] and owner['address']['offset']==slot
                   and not owner.get('volatile'))
            allowed_origin=origin and origin.get('producer') is not None
            if flags and pc not in edge_pcs and owner:
                if temp and allowed_origin and consumer['temp']==origin['producer']:
                    kind='broader_temp_forwarding'
                elif frame:
                    kind='parameter_reload' if owner['address']['kind']=='parameter' else 'frame_reload'
                else:kind='unclassified_redundant_word_load'
            elif equal and not flags:kind='needs_nz_repair'
            else:kind='not_proven_redundant'
            row=dict(pc=pc,slot=slot,classification=kind,body_operation=owner is not None and pc not in edge_pcs,
                     a_matches_home=equal,nz_matches_a=flags,last_barrier=cause,
                     consumer_kind=owner['kind'] if owner else None,
                     block=owner['block'] if owner else None,
                     operation=owner['index'] if owner else None,
                     producer_pc=origin.get('producer_pc') if origin else None)
            if kind in ('broader_temp_forwarding','frame_reload','parameter_reload'):
                row['window']=[[at,b.hex()] for at,b in ins.items() if origin['producer_pc']<=at<=pc]
                row['conditional_saving']=dict(code_bytes=2,instructions=1,cycles=5,stack_reads=2,stack_writes=0)
                row['proof_obligations']=['exact A16 and home byte identity','full N/Z equivalence','no alternate entry or transient S change','retained stores/homes and source-memory order','IRQ/NMI restoration over extended live interval']
                if kind in ('frame_reload','parameter_reload'):row['proof_obligations'].append('new frame-object/parameter forwarding policy; exclude escaping, indexed, volatile and possibly aliased accesses')
            rows.append(row)
            a=value;nz=a;origin=dict(owner or {},producer_pc=pc)
        elif op==0xa9 and not m8:
            a=tuple(('constant',v) for v in code[1:]);nz=a;origin=dict(owner or {},producer_pc=pc)
        elif op==0x83:
            size=1 if m8 else 2
            for i in range(size):memory[code[1]+i]=a[i] if a is not None and not m8 else ('write',pc,i)
        elif op in (0x63,0x69,0xe3,0xe9) and not m8:
            a=(('arithmetic',pc,0),('arithmetic',pc,1));nz=a;origin=dict(owner or {},producer_pc=pc)
        elif op==0xaf and not m8 and owner and owner['kind']=='load' and not owner.get('volatile'):
            a=(('load',pc,0),('load',pc,1));nz=a;origin=dict(owner,producer_pc=pc)
        elif op in (0xc3,0xc9,0xc5):nz=None
        elif op in (0x18,0x38):pass
        else:clear('instruction_clobber_or_memory_barrier')
        # Do not carry facts out of an excluded semantic operation merely because
        # its tail happens to look like an ordinary scalar store.
        if owner and (owner['kind'] in ('call','copy','pointer_offset','unary','cast','address_of') or owner.get('volatile') or owner.get('address',{}).get('indexed') or owner.get('address',{}).get('kind')=='indirect'):
            clear('call_helper_or_alias_barrier')
    return rows


def inventory(directory,facts_path,qualification):
    manifest,records=load(directory)
    q=json.loads(qualification.read_text());facts=json.loads(facts_path.read_text())
    assert facts['schema']==1 and facts['lf_crlf_images_equal']
    assert q['implementation_commit']=='8af3541'
    evidence={}
    def checked(p,h):
        assert digest(p)==h,('changed baseline',p)
        evidence[str(p)]=h
    for name,h in q['comparison_hashes'].items():checked(directory/name,h)
    for name,h in q['compiler_and_fixture_inputs'].items():checked(ROOT/name,h)
    for name,h in manifest['inputs'].items():checked(ROOT/name,h)
    for art in manifest['artifacts']:
        for name,h in art['hashes'].items():checked(Path(art['directory'])/name,h)
    arts={(a['case'],a['mode']):a for a in manifest['artifacts'] if a['compiler']=='actionc'}
    assert len(arts)==len(facts['builds'])==28
    summary=Counter();groups=[];forecasts=[]
    for build in facts['builds']:
        key=build['case'],build['mode'];artifact=arts[key]
        ins=dict(listing(Path(artifact['directory'])/'code.asm'))
        measured=[r for k,r in records.items() if k[:3]==(*key,'actionc')]
        assert measured and all(r['correct'] for r in measured)
        counts=lambda pc:[dict(vector=r['vector'],count=r['instruction_sites'].get(str(pc),0)) for r in measured]
        group=Counter();candidate_pcs=set()
        for routine in build['routines']:
            placed=next(r for r in artifact['routines'] if r['id']==routine['id'])
            assert (placed['address'],placed['size'],placed['fixed_frame'])==(routine['address'],routine['size'],routine['frame_extent'])
            points=interference(routine);homes={t['id']:t['home'] for t in routine['temp_homes']}
            for point,live in points:
                assert all(not overlap(homes[a],homes[b]) for a in live for b in live if a!=b),point
            for e in routine['edges']:
                e['encoding']=edge_code(e,routine['staging_slots'],ins)
                pcs=e['encoding']['pcs'];e['executions']=counts(pcs[0]) if pcs else []
                for row in measured:
                    n=row['instruction_sites'].get(str(pcs[0]),0) if pcs else 0
                    assert all(row['instruction_sites'].get(str(pc),0)==n for pc in pcs)
                group[e['encoding']['form']+'_edges']+=1
                group['edge_assignments']+=len(e['moves'])
                n=sum(c['count'] for c in e['executions'])
                group['edge_assignment_executions']+=n*len(e['moves'])
                for m in e['moves']:
                    c=m['coalescing']=coalescing(m,points,homes)
                    group[c['classification']]+=1;group[c['classification']+'_executions']+=n
                accepted=[]
                for probe in e['layout_probes']:
                    altered=dict(homes)
                    altered.update({c['temp']:c['home'] for c in probe['changes']})
                    collision=any(overlap(altered[a],altered[b]) for _,live in points for a in live for b in live if a!=b)
                    error=probe['verifier_error']
                    assert collision==(error=='overlapping live stack temporaries')
                    if error is None:
                        copies=[i for i,m in enumerate(e['moves']) if m['source'].get('temp') is not None
                                and altered[m['source']['temp']]==altered[m['destination_temp']]]
                        accepted.append(dict(changes=probe['changes'],self_copies=copies))
                e['accepted_layout_probes']=accepted
                e['coalescing_ceiling']=None
                if accepted and e['encoding']['form'] in ('direct_word','acyclic_word'):
                    best=max(accepted,key=lambda p:len(p['self_copies']))
                    copies=best['self_copies']
                    # A last executed load must remain unless the existing tail
                    # reload already restores full A/N/Z. No flag-dead exception.
                    last=e['encoding']['assignments'][-1]['move']
                    loads=sum(i!=last or e['encoding']['reload'] is not None for i in copies)
                    saving=dict(code_bytes=2*(len(copies)+loads),instructions=len(copies)+loads,
                                cycles=5*(len(copies)+loads),stack_reads=2*loads,stack_writes=2*len(copies))
                    e['coalescing_ceiling']=dict(layout=best,per_execution=saving,
                        note='Isolated edge-copy ceiling after a verifier-accepted recoloring; excludes other emitted changes and is not additive with reload forecasts.')
            routine_ins={pc:b for pc,b in ins.items() if routine['address']<=pc<routine['address']+routine['size']}
            routine['reloads']=reloads(routine,routine_ins)
            routine['excluded_byte_stack_loads']=sum(code[0]==0xa3 for code in routine_ins.values())-len(routine['reloads'])
            group['excluded_byte_stack_loads']+=routine['excluded_byte_stack_loads']
            for row in routine['reloads']:
                row['executions']=counts(row['pc']);n=sum(c['count'] for c in row['executions'])
                group['word_stack_loads']+=1;group['word_stack_load_executions']+=n
                group[row['classification']]+=1;group[row['classification']+'_executions']+=n
                if 'conditional_saving' in row:
                    assert row['pc'] not in candidate_pcs;candidate_pcs.add(row['pc'])
        # Coverage comes from the saved runtime decoder, independent of this exporter.
        for row in measured:
            for metric,forms in [('word_edge_sites',{'direct_word','acyclic_word','selective_word','complete_word'}),
                                 ('direct_word_edge_sites',{'direct_word'}),('acyclic_word_edge_sites',{'acyclic_word'}),
                                 ('selective_word_edge_sites',{'selective_word'})]:
                expected={str(e['encoding']['pcs'][0]):next(c['count'] for c in e['executions'] if c['vector']==row['vector'])
                          for r in build['routines'] for e in r['edges'] if e['encoding']['form'] in forms}
                assert row[metric]=={pc:n for pc,n in expected.items() if n},(key,metric)
        for row in measured:
            sites=[s for rt in build['routines'] for s in rt['reloads'] if 'conditional_saving' in s]
            reached=[s for s in sites if row['instruction_sites'].get(str(s['pc']),0)]
            assert all(str(s['pc']) not in row['forwarded_word_load_sites'] for s in sites)
            executions=sum(row['instruction_sites'].get(str(s['pc']),0) for s in sites)
            if sites:
                forecasts.append(dict(case=key[0],mode=key[1],vector=row['vector'],args=row['args'],
                    static_reload_pcs=[s['pc'] for s in sites],reached_reload_pcs=[s['pc'] for s in reached],executions=executions,
                    conditional_delta=dict(code_bytes=-2*len(sites),instructions=-executions,cycles=-5*executions,stack_reads=-2*executions,stack_writes=0,frame_bytes=0),
                    before={field:row[field] for field in ('code_bytes','cycles','instructions','stack_reads','stack_writes','peak_below_entry_s')},
                    note='Requires the new frame/parameter load policy; assumes all listed LDAs alone are removed and layout is otherwise preserved. Not additive with coalescing.'))
        summary.update(group);groups.append(dict(case=key[0],mode=key[1],**dict(sorted(group.items()))))
    for name in ('physical_self_copy','physical_self_copy_executions','broader_temp_forwarding',
                 'broader_temp_forwarding_executions','needs_nz_repair','unclassified_redundant_word_load'):
        summary.setdefault(name,0)
    return dict(schema=1,baseline_commit='8af3541',qualification_commit='e263872',
                scope=dict(action_builds=28,all_records=len(records),action_records=sum(k[2]=='actionc' for k in records)),
                counts='Executions summed across all vectors per incoming I state; debug/release and both I states agree. Static sites counted once per build.',
                caveat='Read-only observations and conditional per-site savings, not post-optimization measurements. Coalescing opportunities interact and are not additive; no frame reduction is forecast.',
                summary=dict(sorted(summary.items())),groups=groups,builds=facts['builds'],reload_forecasts=forecasts,
                evidence_hashes=evidence,facts_sha256=digest(facts_path),qualification_sha256=digest(qualification),
                inventory_tools={str(p.relative_to(ROOT)):digest(p) for p in [Path(__file__),ROOT/'tools/compare65816/test_movement_inventory.py',ROOT/'tools/native65816-runtime-tests/tests/movement_inventory.rs',ROOT/'tests/mir65816_movement_inventory.rs']},
                external_failures=[list(k) for k,r in records.items() if not r['correct']])


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('baseline',type=Path)
    p.add_argument('--facts',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
    p.add_argument('--qualification',type=Path,default=ROOT/'docs/abi/action65816-selective-staging-qualification.json')
    p.add_argument('--check',action='store_true');a=p.parse_args()
    result=inventory(a.baseline,a.facts,a.qualification)
    if a.check:assert json.loads(a.output.read_text())==json.loads(json.dumps(result)),'stale movement inventory'
    else:
        a.output.parent.mkdir(parents=True,exist_ok=True);a.output.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result['summary'],indent=2))


if __name__=='__main__':main()
