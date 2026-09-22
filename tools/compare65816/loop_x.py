#!/usr/bin/env python3
"""Independent typed loop-X admission, frozen images and all-vector forecasts."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import sys
sys.dont_write_bytecode = True
from check_selective_staging import all_instructions, relocate
from inventory_registers import loops, verify_snapshot
from inventory_scalar_dp import admission as scalar_admission, digest
from delta import load

ROOT = Path(__file__).resolve().parents[2]


def canonical(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def candidate(r):
    reasons, words, _ = scalar_admission(r)
    reasons = [v for v in reasons if v != 'existing_dp_path']
    if reasons: return dict(routine=r['id'], rejections=reasons)
    if not words or any(t['home']['kind'] != 'dp' for t in words):
        return dict(routine=r['id'], rejections=['not_scalar_dp'])
    natural = loops(r)
    if len(natural) != 1: return dict(routine=r['id'], rejections=['loop_count'])
    loop = natural[0]; blocks = {b['id']: b for b in r['blocks']}
    header, body = blocks[loop['header']], blocks[loop['latch']]
    incoming = [e for e in r['edges'] if e['target_block'] == header['id']]
    ops = [o for o in header['ops'] if not o.get('terminator')]
    if (loop['blocks'] != sorted([header['id'], body['id']]) or len(ops) != 1 or
        ops[0]['kind'] != 'compare' or header['terminator_kind'] != 'branch' or
        body['terminator_kind'] != 'goto' or body['params'] or len(incoming) != 2 or
        any(e['moves'] for e in r['edges'] if e['block'] == header['id']) or
        len([e for e in r['edges'] if e['target_block'] == body['id']]) != 1 or
        any(e['arm'] != 'goto' for e in incoming)):
        return dict(routine=r['id'], rejections=['loop_shape'])
    compare = ops[0]
    for param in loop['parameters']:
        p = param['temp']; t = param['type']; update = param['update']
        if (t.get('signed') is not False or t.get('ordinary') is not True or
            p != header['params'][-1] or update is None or update['block'] != body['id'] or
            update['binary'] != 'add'): continue
        op = next(o for o in body['ops'] if o['index'] == update['operation'])
        cv, uv = compare['values'], op['values']
        if (compare['signed'] or compare['predicate'] not in ('lt', 'le') or
            cv[0].get('temp') != p or cv[1]['kind'] != 'immediate' or
            uv[0].get('temp') != p or uv[1]['kind'] != 'immediate' or uv[1]['value'] != 1): continue
        threshold = cv[1]['value'] + (compare['predicate'] == 'le')
        if threshold > 65535: continue
        uses = [(b['id'], o['index']) for b in r['blocks'] for o in b['ops']
                if not o.get('terminator') and p in o['uses']]
        if sorted(uses) != sorted([(header['id'], compare['index']), (body['id'], op['index'])]): continue
        if any(p in b['terminator_uses'] for b in r['blocks']): continue
        boolean = compare['definition']
        if any(boolean in o['uses'] for b in r['blocks'] for o in b['ops'] if not o.get('terminator')): continue
        if [b['id'] for b in r['blocks'] if boolean in b['terminator_uses']] != [header['id']]: continue
        if any(m['source'].get('temp') == boolean for e in r['edges'] for m in e['moves']): continue
        home = param['home']; q = param['backedge_source']['temp']
        homes = {t['id']: t['home'] for t in r['temp_homes']}
        if any(homes[o['definition']] == home for b in (header, body) for o in b['ops'] if o['definition'] is not None): continue
        if not param['closed_operation_conflicts']: continue
        return dict(routine=r['id'], rejections=[], header=header['id'], body=body['id'],
                    param=p, update=q, home=home, threshold=threshold,
                    compare_range=compare['range'], update_range=op['range'],
                    incoming=incoming, closed_operation_conflicts=param['closed_operation_conflicts'])
    return dict(routine=r['id'], rejections=['no_private_counter'])


def sites(trial, ins):
    t = copy.deepcopy(trial); h = t['home']['offset']; at = t['compare_range'][0]
    if ins[at] == b'\xc2\x20': at += 2
    # Both admitted predicates have one load and one comparison.
    op = ins[at]
    if op[0] == 0xa9:
        assert ins[at+3] == bytes([0xc5, h]); removed = at+3; branch = at+5
    else:
        assert op == bytes([0xa5, h]) and ins[at+2][0] == 0xc9
        removed = at+2; branch = at+5
    assert ins[branch][0] in (0x90, 0xb0)
    start, end = t['update_range']
    loads = [pc for pc, code in ins.items() if start <= pc < end and code == bytes([0xa5, h])]
    assert len(loads) == 1
    tails = []
    for e in t['incoming']:
        end = e['transfer_pc']; assert ins[end-2] == bytes([0x85, h]) or ins[end-2] == bytes([0xa5, h])
        tails.append(end-2)
    t.update(compare_pc=at, removed_pc=removed, branch_pc=branch, load_pc=loads[0],
             refresh_after=sorted(tails), before={str(pc):ins[pc].hex() for pc in [at, removed, branch, loads[0], *tails]})
    return t


def transform(before, t):
    instructions = all_instructions(before); old = dict(instructions)
    for pc, code in t['before'].items(): assert old[int(pc)].hex() == code
    replacements = {t['compare_pc']: b'\xe0'+t['threshold'].to_bytes(2,'little'),
                    t['load_pc']: b'\x8a', t['branch_pc']: b'\x90'+old[t['branch_pc']][1:]}
    mapping = {}; emitted = []; refresh = {}; cursor = None; prev_end = None
    for pc, code in instructions:
        if prev_end is None or pc != prev_end: cursor = pc if cursor is None else cursor + pc-prev_end
        prev_end = pc+len(code)
        if pc == t['removed_pc']: continue
        mapping[pc] = cursor
        new = replacements.get(pc, code)
        emitted.append((pc,cursor,new)); cursor += len(new)
        if pc in t['refresh_after']:
            refresh[pc] = cursor; emitted.append((None,cursor,b'\xaa')); cursor += 1
    ranges = [(s['address'], s['address']+len(s['bytes'])) for s in before['segments'] if s['executable']]
    # Boundaries may coincide with the start of the next routine.
    for lo, hi in ranges:
        if hi not in mapping:
            mapping[hi] = max(at+len(code) for pc,at,code in emitted if pc is not None and lo<=pc<hi)
    def remap(pc):
        assert pc != t['removed_pc'], 'target in replaced comparison'
        return mapping.get(pc, pc)
    new = [(at, code if pc is None else relocate(pc,code,at,remap)) for pc,at,code in emitted]
    image = copy.deepcopy(before);image['entry']=remap(image['entry'])
    for s in image['segments']:
        if not s['executable']:continue
        lo,hi=s['address'],s['address']+len(s['bytes']);s['address']=remap(lo)
        s['bytes']=list(b''.join(code for at,code in new if remap(lo)<=at<remap(hi)))
        assert len(s['bytes'])==remap(hi)-remap(lo)
    for r in image['routines']:
        lo=r['address'];r['address']=remap(lo);r['size']=remap(lo+r['size'])-r['address']
    return image,new,remap,refresh


def measurement(old,t=None,remap=lambda p:p,refresh=None):
    out=copy.deepcopy(old);out.update(x_forwarded_loads=0,x_forwarded_load_sites={})
    if t is None:return out
    counts=old['instruction_sites'];h=counts.get(str(t['compare_pc']),0);u=counts.get(str(t['load_pc']),0)
    e=sum(counts.get(str(pc),0) for pc in t['refresh_after'])
    out['code_bytes']-=1;out['cycles']-=4*h+2*u-2*e;out['instructions']+=e-h;out['dp_reads']-=2*(h+u)
    for field,value in old.items():
        if field.endswith('_sites'):
            out[field]={str(remap(int(pc))):n for pc,n in value.items() if int(pc)!=t['removed_pc']}
    for pc,at in refresh.items():
        n=counts.get(str(pc),0)
        if n:out['instruction_sites'][str(at)]=n
    out['x_forwarded_loads']=u
    if u:out['x_forwarded_load_sites']={str(remap(t['load_pc'])):u}
    return out


def freeze(directory,facts):
    verify_snapshot(directory,ROOT/'docs/benchmarks/65816-scalar-dp/after')
    manifest,records=load(directory);builds=[]
    for artifact in manifest['artifacts']:
        for name,sha in artifact['hashes'].items(): assert digest(Path(artifact['directory'])/name)==sha
    for b in facts['builds']:
        a,=[a for a in manifest['artifacts'] if (a['case'],a['mode'],a['compiler'])==(b['case'],b['mode'],'actionc')]
        image=json.loads(Path(a['image']).read_text());ins=dict(all_instructions(image))
        trials=[candidate(r) for r in b['routines']];selected=[t for t in trials if not t['rejections']]
        assert len(selected)<=1
        t=sites(selected[0],ins) if selected else None
        expected,_,remap,refresh=transform(image,t) if t else (image,[],lambda p:p,{})
        projections=[dict(vector=r['vector'],forecast=measurement(r,t,remap,refresh)) for k,r in records.items() if k[:3]==(b['case'],b['mode'],'actionc')]
        builds.append(dict(case=b['case'],mode=b['mode'],trials=trials,selected=t,expected_image_sha256=canonical(expected),projections=projections))
    assert sum(b['selected'] is not None for b in builds)==1
    return dict(schema=1,comparison_hashes={f:digest(directory/f) for f in ('manifest.json','debug.json','release.json')},builds=builds)


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('directory',type=Path);p.add_argument('--facts',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();result=freeze(a.directory,json.loads(a.facts.read_text()));result['facts_sha256']=digest(a.facts);a.output.parent.mkdir(parents=True,exist_ok=True);a.output.write_text(json.dumps(result,indent=2)+'\n')
    print('Frozen 28 images, 58 admission decisions and 132 counter forecasts')


if __name__=='__main__':main()
