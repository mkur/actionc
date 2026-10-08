#!/usr/bin/env python3
"""Investigate typed argument/result flow without changing compiler selection."""
import argparse
from collections import Counter
import csv
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess

from exec_call_audit import FILES as CALL_FILES, ROOT, compiler_inputs, csv_text, require, touches_home
from exec_record_baseline import digest, read, save, verify_inputs

AUDIT = ROOT/'docs/benchmarks/65816-exec-call-audit'
PROFILE = 'optimized-release'
FILES = (*CALL_FILES,'probe.flow.json')


def collect(base, output):
    verify_inputs(base)
    before = compiler_inputs()
    target = ROOT/'target/exec-call-audit/rust-target'
    binary = target/'debug/actionc-exec-record-probe'
    build = ['cargo','build','--locked','--manifest-path',str(ROOT/'tools/compare65816/record_probe/Cargo.toml'),
             '--features','flow-analysis','--target-dir',str(target),'-j2']
    env = dict(os.environ,CARGO_INCREMENTAL='0',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_DEV_OPT_LEVEL='3')
    subprocess.run(build,check=True,cwd=ROOT,env=env)
    output.mkdir(parents=True,exist_ok=True)
    command = [str(binary),str(base/'profiles'/PROFILE),str(base/'exec'),str(output/'probe'),'true','false',str(ROOT)]
    subprocess.run(command,check=True,cwd=ROOT)
    original = read(AUDIT/'provenance.json')
    for name in CALL_FILES:
        require(digest(output/name)==original['profiles'][PROFILE]['artifacts'][name], 'Original audit identity mismatch: '+name)
    require(before==compiler_inputs(),'Compiler/probe source drift')
    verify_inputs(base)
    save(output/'provenance.json',dict(schema=1,compiler_revision=original['compiler_revision'],exec_revision=original['exec_revision'],
        profile=PROFILE,rust=subprocess.check_output(['rustc','--version'],cwd=ROOT,text=True).strip(),
        build_command=build,build_environment={k:env[k] for k in ('CARGO_INCREMENTAL','CARGO_PROFILE_DEV_DEBUG','CARGO_PROFILE_DEV_OPT_LEVEL')},
        command=command,compiler_inputs=before,probe_binary_sha256=digest(binary),
        frozen_inputs_sha256=digest(base/'inputs.json'),audit_provenance_sha256=digest(AUDIT/'provenance.json'),
        artifacts={name:digest(output/name) for name in FILES}))


def is_temp(value, tid):
    return value.get('kind')=='temp' and value['id']==tid


def capture_rows(data):
    return list(csv.DictReader(io.StringIO(data.decode(),newline='')))


def comparison(op, tid):
    require(op['kind']=='Compare','Expected typed comparison')
    left,right = is_temp(op['left'],tid),is_temp(op['right'],tid)
    require(left != right,'Sole comparison use has ambiguous operand')
    other = op['right'] if left else op['left']
    zero = other['kind']=='constant' and other['value']==0
    return dict(width=op['width'],operation=op['operation'],signed=op['signed'],side='left' if left else 'right',
                other_kind=other['kind'],zero=zero,
                scalar_zero_test=op['width'] in (1,2) and op['operation'] in ('Eq','Ne') and zero)


def addressed_temps(address):
    return [v['id'] for v in (address.get('base_value'),(address.get('index') or {}).get('value'))
            if v and v['kind']=='temp']


def home_read_bytes(routine, temp, point):
    """Encoded reads of the actual canonical home at the consumer's source site."""
    total = 0
    for e in routine.get('instructions',[]):
        if e['source']!=list(point) or e['control'] in ('call','forward','return'):
            continue
        if any(m['access']=='read' and touches_home(dict(m,access='write'),e,temp.get('home'),routine['frame']) for m in e['memory']):
            total += e['end']-e['start']
    return total


def private_source_reason(producer, routine, gap):
    """Conservative candidate screen, not the compiler's admission decision."""
    if producer['kind']!='Load':
        return 'not_load'
    a = producer['address']
    if producer['volatile'] or a['kind'] not in ('immutable_parameter','private_frame'):
        return 'nonprivate_or_volatile'
    if a['indexed'] or a['displacement']!=0:
        return 'noncanonical_view'
    homes = routine['parameters'] if a['kind']=='immutable_parameter' else routine['objects']
    home = next(h for h in homes if h['id']==a['id'])
    if a['kind']=='private_frame' and any(p['object']==a['id'] for p in routine['parameters']):
        return 'parameter_frame_alias'
    if home['width']!=producer['width']:
        return 'partial_home'
    for b in routine['blocks']:
        for op in b['ops']:
            addresses = [op.get('address'),op.get('source'),op.get('destination')]
            if any(x and (x['kind'],x['id'])==(a['kind'],a['id']) for x in addresses):
                if op['kind'] in ('AddressOf','Copy') or op.get('volatile') or op['width']!=producer['width'] or any(x and (x['kind'],x['id'])==(a['kind'],a['id']) and (x['indexed'] or x['displacement']!=0) for x in addresses):
                    return 'addressed_or_mixed_views'
                if op['kind']=='Store' and a['kind']=='immutable_parameter':
                    return 'parameter_write'
    if any(op['kind'] in ('Call','Store','Copy') or op.get('volatile') for op in gap):
        return 'intervening_barrier'
    return 'bounded_private_candidate'


def investigate(typed, flow, captures):
    require(flow.get('schema')==1,'Unknown flow schema')
    facts = {r['id']:r for r in flow['routines']}
    require(len(facts)==len(flow['routines']) and facts.keys()=={r['id'] for r in typed['routines']},'Typed flow routine mismatch')
    capture = {(r['routine'],int(r['temp'])):r for r in captures if r['profile']==PROFILE}
    results,arguments = [],[]
    counts,compare_counts,store_counts,cast_counts,argument_counts = (Counter() for _ in range(5))
    for r in typed['routines']:
        f = facts[r['id']]
        require(f['name']==r['name'],'Flow routine identity mismatch')
        blocks = {b['id']:b['ops'] for b in f['blocks']}
        temps = {t['id']:t for t in r['temps']}
        calls = {(c['block'],c['index']):c for c in r['calls']}
        for t in r['temps']:
            row = capture.get((r['name'],t['id']))
            if row is None or not int(row['capture_write_code_bytes']):
                continue
            footprint = int(row['capture_write_code_bytes'])
            p = t['producer']
            if 'index' not in p:
                continue
            producer = blocks[p['block']][p['index']]
            require(producer['kind']==p['kind'],'Producer kind mismatch')
            sole = len(t['uses'])==1
            adjacent = sole and t['uses'][0]['block']==p['block'] and t['uses'][0]['index']==p['index']+1
            if p['kind']=='Call' and adjacent:
                use = t['uses'][0]
                consumer = blocks[use['block']][use['index']] if use['index']<len(blocks[use['block']]) else dict(kind=use['kind'])
                require(consumer['kind']==use['kind'],'Consumer kind mismatch')
                reads = home_read_bytes(r,t,(use['block'],use['index']))
                details = {}
                if consumer['kind']=='Compare':
                    details = comparison(consumer,t['id'])
                    dest = temps[consumer['dest']]
                    details['sole_adjacent_branch'] = len(dest['uses'])==1 and dest['uses'][0]['kind']=='Branch' and dest['uses'][0]['block']==use['block'] and dest['uses'][0]['index']==use['index']+1
                    compare_counts[f"{details['width']}/{details['operation']}/{details['signed']}/{details['other_kind']}/{details['zero']}"] += 1
                    counts['scalar_zero_test_results'] += details['scalar_zero_test']
                    counts['scalar_zero_test_branch_results'] += details['scalar_zero_test'] and details['sole_adjacent_branch']
                    counts['scalar_zero_test_capture_bytes'] += footprint if details['scalar_zero_test'] else 0
                    counts['scalar_zero_test_home_read_bytes'] += reads if details['scalar_zero_test'] else 0
                    counts['scalar_zero_test_without_home_reads'] += details['scalar_zero_test'] and not reads
                elif consumer['kind']=='Store':
                    details = dict(value_is_result=is_temp(consumer['value'],t['id']),destination=consumer['address']['kind'],
                        indexed=consumer['address']['indexed'],volatile=consumer['volatile'],address_depends_on_result=t['id'] in addressed_temps(consumer['address']))
                    store_counts[f"{details['value_is_result']}/{details['destination']}/{details['indexed']}/{details['volatile']}"] += 1
                    private = details['value_is_result'] and details['destination']=='private_frame' and not details['indexed'] and not details['volatile']
                    counts['private_frame_result_assignments'] += private
                    counts['private_frame_assignment_capture_bytes'] += footprint if private else 0
                    counts['private_frame_assignment_home_read_bytes'] += reads if private else 0
                elif consumer['kind']=='Cast':
                    details = dict(source_width=consumer['from'],destination_width=consumer['width'],kind=consumer['cast_kind'],source_signed=consumer['from_signed'],
                        shares_canonical_home=t.get('home') is not None and t['home']==temps[consumer['dest']].get('home'))
                    cast_counts[f"{consumer['from']}/{consumer['width']}/{consumer['cast_kind']}/{consumer['from_signed']}"] += 1
                    identity = consumer['from']==consumer['width']==3
                    counts['same_width_three_byte_casts'] += identity
                    counts['same_width_three_byte_casts_without_reads'] += identity and not reads
                    counts['same_width_three_byte_casts_sharing_home'] += identity and details['shares_canonical_home']
                elif consumer['kind']=='Call':
                    c = calls[(use['block'],use['index'])]
                    details = dict(arguments=len(c['arguments']),target_kind=c['target_kind'],positions=[i for i,a in enumerate(c['arguments']) if is_temp(a['value'],t['id'])])
                results.append(dict(routine=r['name'],temp=t['id'],width=t['width'],consumer=consumer['kind'],capture_bytes=footprint,home_read_bytes=reads,
                    block=p['block'],index=p['index'],details=json.dumps(details,sort_keys=True)))
                counts['adjacent_captured_results'] += 1
                counts['adjacent_result_capture_bytes'] += footprint
            if sole and t['uses'][0]['kind']=='Call':
                use = t['uses'][0]
                c = calls[(use['block'],use['index'])]
                # Indirect callee operands are call uses, not outgoing arguments.
                positions = [i for i,a in enumerate(c['arguments']) if is_temp(a['value'],t['id'])]
                if not positions:
                    continue
                gap = blocks[p['block']][p['index']+1:use['index']] if use['block']==p['block'] else []
                reason = private_source_reason(producer,f,gap) if use['block']==p['block'] else 'cross_block'
                reads = home_read_bytes(r,t,(use['block'],use['index']))
                arguments.append(dict(routine=r['name'],temp=t['id'],width=t['width'],producer=p['kind'],capture_bytes=footprint,home_read_bytes=reads,
                    source=producer.get('address',{}).get('kind',''),adjacent=adjacent,arguments=len(c['arguments']),position=positions[0],
                    target_kind=c['target_kind'],private_source_screen=reason))
                counts['sole_call_arguments_with_stores'] += 1
                counts['sole_argument_capture_bytes'] += footprint
                counts['adjacent_sole_arguments_with_stores'] += adjacent
                counts['bounded_private_scalar_arguments'] += reason=='bounded_private_candidate' and t['width'] in (1,2) and c['target_kind'] in ('direct','helper','runtime')
                private = reason=='bounded_private_candidate' and t['width'] in (1,2) and c['target_kind'] in ('direct','helper','runtime')
                counts['bounded_private_scalar_capture_bytes'] += footprint if private else 0
                counts['bounded_private_scalar_home_read_bytes'] += reads if private else 0
                counts['bounded_private_scalar_multi_argument_calls'] += private and len(c['arguments'])>1
                argument_counts[f"{t['width']}/{p['kind']}/{producer.get('address',{}).get('kind','')}/{reason}"] += 1
    return dict(counts=dict(counts),comparisons=dict(compare_counts),stores=dict(store_counts),casts=dict(cast_counts),arguments=dict(argument_counts)),results,arguments


def report(output,destination,check):
    provenance = read(output/'provenance.json')
    require(digest(AUDIT/'provenance.json')==provenance['audit_provenance_sha256'],'Original audit provenance drift')
    for name in FILES:
        require(digest(output/name)==provenance['artifacts'][name],'Flow input hash mismatch: '+name)
    for name,expected in read(AUDIT/'evidence-sha256.json').items():
        require(digest(AUDIT/name)==expected,'Original audit evidence drift: '+name)
    captures = capture_rows(gzip.decompress((AUDIT/'captures.csv.gz').read_bytes()))
    result,results,arguments = investigate(read(output/'probe.calls.json'),read(output/'probe.flow.json'),captures)
    original = read(AUDIT/'results.json')['profiles'][PROFILE]['cohorts']
    require(result['counts']['adjacent_captured_results']==original['adjacent_result_values_with_capture_writes']
            and result['counts']['sole_call_arguments_with_stores']==original['sole_call_argument_values_with_capture_writes']
            and result['counts']['adjacent_result_capture_bytes']==original['adjacent_result_capture_write_code_bytes']
            and result['counts']['sole_argument_capture_bytes']==original['sole_call_argument_capture_write_code_bytes'], 'Frozen cohort mismatch')
    result.update(schema=1,profile=PROFILE,compiler_revision=provenance['compiler_revision'],exec_revision=provenance['exec_revision'],
        scope='Read-only structural investigation; candidate screens are not compiler admission or savings estimates.')
    provenance['report_tools'] = {str(p.relative_to(ROOT)):digest(p) for p in (Path(__file__),Path(__file__).with_name('test_exec_call_flow.py'))}
    artifacts = {'results.json':(json.dumps(result,indent=2)+'\n').encode(),'provenance.json':(json.dumps(provenance,indent=2)+'\n').encode(),
        'result-cases.csv.gz':gzip.compress(csv_text(results).encode(),mtime=0),'argument-cases.csv.gz':gzip.compress(csv_text(arguments).encode(),mtime=0)}
    artifacts['evidence-sha256.json'] = (json.dumps({n:hashlib.sha256(data).hexdigest() for n,data in artifacts.items()},indent=2)+'\n').encode()
    for name,data in artifacts.items():
        path = destination/name
        if check:
            require(path.is_file() and path.read_bytes()==data,'Published flow report differs: '+name)
        else:
            destination.mkdir(parents=True,exist_ok=True)
            path.write_bytes(data)
    print('Flow report verified' if check else 'Flow report written',destination)
    print(json.dumps(result['counts']))


if __name__=='__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('operation',choices=('collect','report'))
    p.add_argument('--base',type=Path,default=ROOT/'target/record-placement-stage0')
    p.add_argument('--output',type=Path,default=ROOT/'target/exec-call-flow')
    p.add_argument('--destination',type=Path,default=AUDIT/'argument-result-flow')
    p.add_argument('--check',action='store_true')
    args = p.parse_args()
    if args.operation=='collect':
        require(not args.check,'--check applies to report')
        collect(args.base.resolve(),args.output.resolve())
    else:
        report(args.output.resolve(),args.destination.resolve(),args.check)
