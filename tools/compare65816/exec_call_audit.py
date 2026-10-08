#!/usr/bin/env python3
"""Audit frozen Exec call costs using typed observations and final machine bytes.

Static footprints and policy cohorts are not savings estimates or runtime costs.
This observer does not change compiler strategy, the ABI, or frozen acceptance.
"""
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
import sys

sys.dont_write_bytecode = True
from exec_record_baseline import PROFILES, digest, guard_ranges, instructions, inventory, read, save, verify_inputs

ROOT = Path(__file__).resolve().parents[2]
STAGE7 = ROOT / 'docs/benchmarks/65816-record-placement-stage7/results.json'
FILES = ('probe.image.json', 'probe.inventory.json', 'probe.placement.json', 'probe.calls.json')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def csv_text(rows):
    if not rows:
        return ''
    stream = io.StringIO(newline='')
    writer = csv.DictWriter(stream, fieldnames=list(rows[0]), lineterminator='\n')
    writer.writeheader()
    writer.writerows(rows)
    return stream.getvalue()


def compiler_inputs():
    paths = [*ROOT.joinpath('src').rglob('*.rs'), *ROOT.joinpath('runtime/65816').glob('*'),
             ROOT/'Cargo.toml', ROOT/'Cargo.lock', ROOT/'rust-toolchain.toml',
             *ROOT.joinpath('tools/compare65816/record_probe/src').glob('*.rs'),
             ROOT/'tools/compare65816/record_probe/Cargo.toml', ROOT/'tools/compare65816/record_probe/Cargo.lock']
    return {str(p.relative_to(ROOT)): digest(p) for p in paths if p.is_file()}


def collect(base, output):
    verify_inputs(base)
    before = compiler_inputs()
    binary = output/'rust-target/debug/actionc-exec-record-probe'
    env = dict(os.environ, CARGO_INCREMENTAL='0', CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_DEV_OPT_LEVEL='3')
    build = ['cargo', 'build', '--locked', '--manifest-path', str(ROOT/'tools/compare65816/record_probe/Cargo.toml'),
             '--features', 'call-analysis', '--target-dir', str(output/'rust-target'), '-j2']
    subprocess.run(build, check=True, cwd=ROOT, env=env)
    record = dict(schema=1, compiler_revision=subprocess.check_output(['git','rev-parse','HEAD'], cwd=ROOT, text=True).strip(),
                  rust=subprocess.check_output(['rustc','--version'], cwd=ROOT, text=True).strip(),
                  compiler_inputs=before, probe_binary_sha256=digest(binary), build_command=build,
                  build_environment={k:env[k] for k in ('CARGO_INCREMENTAL','CARGO_PROFILE_DEV_DEBUG','CARGO_PROFILE_DEV_OPT_LEVEL')},
                  frozen_inputs_sha256=digest(base/'inputs.json'), exec_revision=read(base/'inputs.json')['exec']['revision'], profiles={})
    for profile, (optimize, guards) in PROFILES.items():
        generated, destination = base/'profiles'/profile, output/profile
        destination.mkdir(parents=True, exist_ok=True)
        generated_hashes = {str(p.relative_to(generated)):digest(p) for p in generated.rglob('*')
                            if p.is_file() and (p.suffix in ('.act','.inc') or p.name == 'layout.json')}
        command = [str(binary), str(generated), str(base/'exec'), str(destination/'probe'),
                   str(optimize).lower(), str(guards).lower(), str(ROOT)]
        subprocess.run(command, check=True, cwd=ROOT)
        require(generated_hashes == {name:digest(generated/name) for name in generated_hashes}, 'Generated inputs changed')
        image_hash = digest(destination/'probe.image.json')
        expected = read(STAGE7)['profiles'][profile]['artifact_hashes']['probe.image.json']
        require(image_hash == expected, 'Current image differs from stage 7: '+profile)
        ir = read(destination/'probe.inventory.json')
        record['profiles'][profile] = dict(command=command, generated_inputs=generated_hashes,
            actual_source_files={name:digest(Path(name)) for name in ir['source_paths']},
            image_sha256=image_hash, stage7_image_sha256=expected, stage7_image_identical=True,
            artifacts={name:digest(destination/name) for name in FILES})
    require(before == compiler_inputs() and digest(binary) == record['probe_binary_sha256'], 'Compiler or probe drift')
    verify_inputs(base)
    save(output/'provenance.json', record)


def location_access(effect, instruction, routine):
    """Classify physical coordinates, without treating aliasing as permission."""
    if effect['kind'] == 'dp':
        return 'dp_scratch' if 0x80 <= effect['offset'] and effect['offset']+effect['bytes'] <= 0xc0 else 'dp_other'
    if effect['kind'] != 'stack':
        return effect['kind']
    start = effect['offset'] - instruction['depth']
    end = start + effect['bytes']
    if end <= 1-routine['frame']:
        return 'outgoing_stack'
    if start <= 0 and end <= 1:
        return 'invocation_stack'
    return 'incoming_or_transfer_stack'


def touches_home(effect, instruction, home, frame):
    if not home or effect['access'] != 'write':
        return False
    if home['kind'] == 'dp':
        return effect['kind'] == 'dp' and home['offset'] <= effect['offset'] and effect['offset']+effect['bytes'] <= home['offset']+home['width']
    start = effect.get('offset', 0) - instruction['depth']
    return effect['kind'] == 'stack' and home['offset']-frame <= start and start+effect['bytes'] <= home['offset']-frame+home['width']


def diagnostic(temp):
    """First matching census category, not an internal allocator refusal code."""
    if temp['home'] is None:
        return 'no_materialized_home'
    if temp['home']['kind'] == 'dp':
        return 'direct_page_home'
    if temp['crossings']:
        return 'lifetime_crosses_call'
    if temp['producer']['kind'] == 'Call':
        return 'call_result'
    if temp['width'] not in (2,3):
        return 'outside_mixed_residence_widths'
    if len(temp['uses']) != 1:
        return 'multiple_or_zero_uses'
    use = temp['uses'][0]
    if use['block'] != temp['producer']['block']:
        return 'cross_block_use'
    if use['kind'] == 'Call':
        return 'call_argument'
    if temp['producer'].get('volatile'):
        return 'volatile_capture'
    return 'other_stack_backed'


def example_listing(image, ir, typed, captures):
    """Curated report excerpts; names only select examples, never compiler policy."""
    physical = {r['id']:r for r in image['routines']}
    logical = {r['id']:r for r in ir['routines']}
    footprint = {(c['routine'],c['temp']):c['capture_write_code_bytes'] for c in captures}
    decoded = list(instructions(image))
    selectors = (
        ('Result followed by comparison', 'M_SDFSFILE_OPEN_',
         lambda t: t['producer']['kind']=='Call' and len(t['uses'])==1 and t['uses'][0]['kind']=='Compare'),
        ('Captured argument before a call', 'M_COOKEDLINE_RECALL_',
         lambda t: t['producer']['kind']!='Call' and len(t['uses'])==1 and t['uses'][0]['kind']=='Call'),
        ('Pointer live across a call', 'M_SDFSFILE_MEASURE_',
         lambda t: t['pointer'] and bool(t['crossings'])),
        ('Reserved return home with no capture', 'M_TASKPOLICY_',
         lambda t: t['producer']['kind']=='Call' and t['home'] and len(t['uses'])==1 and t['uses'][0]['kind']=='Return'),
    )
    lines = ['; Exact optimized-release excerpts from the audited frozen compiler image.',
             '; Store footprints exclude loads and mode changes. Homes may be unused reservations.', '']
    for title,prefix,predicate in selectors:
        found = [(r,t) for r in typed['routines'] if r['name'].startswith(prefix) for t in r['temps']
                 if predicate(t) and (footprint.get((r['name'],t['id']),0)==0 if title.startswith('Reserved')
                                      else footprint.get((r['name'],t['id']),0)>0)]
        require(bool(found), 'Missing curated example: '+title)
        r,t = found[0]
        p,l = physical[r['id']],logical[r['id']]
        producer = (t['producer']['block'],t['producer'].get('index'))
        points = [producer]
        if t['crossings']:
            points.append(tuple(t['crossings'][0]))
        else:
            points.append((t['uses'][0]['block'],t['uses'][0]['index']))
        lines.extend(['; '+title, '; '+r['name'], '; temp '+str(t['id'])+': '+json.dumps(t,sort_keys=True),
                      '; actual capture store instructions: '+str(footprint[(r['name'],t['id'])])+' code bytes'])
        for block,index in dict.fromkeys(points):
            spans = [s for s in l['spans'] if (s['block'],s['index'])==(block,index)]
            require(len(spans)==1, 'Missing example source span')
            s = spans[0]
            lines.append(f"; block {block}, index {index}, {s['kind']}, {s['end']-s['start']} bytes")
            for pc,data,disassembly in decoded:
                if p['address']+s['start'] <= pc < p['address']+s['end']:
                    lines.append(f"{pc:06X}  {' '.join(f'{b:02X}' for b in data):<12} {disassembly}")
        lines.append('')
    return '\n'.join(lines)


def audit_profile(image, ir, typed, placement):
    require(typed.get('schema') == 1, 'Unknown typed census schema')
    summary, _ = inventory(image, ir)
    physical = {r['id']:r for r in image['routines']}
    logical = {r['id']:r for r in ir['routines']}
    observed = {r['id']:r for r in typed['routines']}
    require(len(observed) == len(typed['routines']) and physical.keys() == observed.keys(), 'Typed routine census mismatch')
    decoded = {pc:(data,text) for pc,data,text in instructions(image)}
    guards = {pc for start,end in guard_ranges(image) for pc in range(start,end)}
    calls, routines, helpers, captures = [], [], [], []
    transfer_count, diagnostic_counts, traffic, request_counts = Counter(), Counter(), Counter(), Counter()
    cohorts = Counter()
    for rid, r in observed.items():
        p, l = physical[rid], logical[rid]
        require(r['name'] == p['name'] and r['frame'] == p['fixed_frame'] and r['local_peak'] == p['local_stack_peak'], 'Physical metadata mismatch')
        require(r['spill'] == p['spill_bytes'], 'Spill metadata mismatch')
        temps = {t['id']:t for t in r['temps']}
        require(len(temps) == len(r['temps']), 'Duplicate typed temp')
        homes = {t['id']:t for t in p['temporaries']}
        require({t['id'] for t in r['temps'] if t['home']} == homes.keys(), 'Materialized temp census mismatch')
        for t in r['temps']:
            if t['home']:
                h = t['home']
                expected = dict(kind='direct_page',offset=h['offset']) if h['kind']=='dp' else dict(kind='stack',displacement=h['offset'])
                require(h['width']==t['width']==homes[t['id']]['size'] and expected==homes[t['id']]['home'], 'Typed home geometry mismatch')
        require(sum(e['control']=='call' for e in r['instructions'])==len(p['calls']), 'Physical/image call census mismatch')
        source_effects = {}
        covered = set()
        for e in r['instructions']:
            require(0 <= e['start'] <= e['end'] <= p['size'], 'Effect outside routine')
            if e['start'] == e['end']:
                continue
            extent = set(range(e['start'],e['end']))
            require(not covered & extent, 'Overlapping selected effects')
            covered |= extent
            require(p['address']+e['start'] in decoded, 'Effect not at instruction boundary')
            require(e['end'] == p['size'] or p['address']+e['end'] in decoded, 'Effect ends inside instruction')
            if e['source'] is not None:
                source_effects.setdefault(tuple(e['source']), []).append(e)
            if e['control'] in ('call','forward'):
                transfer_count[e['control']] += 1
            # ABI call/return summaries include callee argument observations and
            # unknown clobbers. They are not physical accesses by that opcode.
            if e['control'] in ('call','forward','return') or p['address']+e['start'] in guards:
                continue
            for effect in e['memory']:
                if effect['access'] != 'may_write' and effect['bytes'] is not None:
                    traffic[location_access(effect,e,r)+'_'+effect['access']+'_bytes'] += effect['bytes']
        require(len(covered) == p['size'], 'Selected effects do not cover executable bytes')
        for req in r['requests']:
            request_counts[str(req['request'])+'/'+str(req['accepted'])] += 1
        capture_by_temp = {}
        argument_ids = {a['value']['id'] for c in r['calls'] for a in c['arguments'] if a['value']['kind']=='temp'}
        for tid,t in temps.items():
            diagnostic_counts[diagnostic(t)] += 1
            producer = t['producer']
            writes = [e for e in source_effects.get((producer['block'],producer.get('index')), [])
                      if e['control'] not in ('call','forward','return') and any(touches_home(m,e,t['home'],r['frame']) for m in e['memory'])]
            capture_bytes = sum(e['end']-e['start'] for e in writes)
            capture_by_temp[tid] = capture_bytes
            if tid in argument_ids or producer['kind'] == 'Call' or t['crossings']:
                captures.append(dict(routine=r['name'],temp=tid,width=t['width'],pointer=t['pointer'],producer=producer['kind'],
                    source=producer.get('address',{}).get('kind',''),home=(t['home'] or {}).get('kind','none'),uses=len(t['uses']),
                    sole_consumer=t['uses'][0]['kind'] if len(t['uses'])==1 else '',
                    adjacent_consumer=bool(len(t['uses'])==1 and t['uses'][0]['block']==producer['block'] and t['uses'][0]['index']==producer.get('index',-2)+1),
                    call_crossings=len(t['crossings']),call_argument=tid in argument_ids,result_of_call=producer['kind']=='Call',
                    capture_write_code_bytes=capture_bytes,diagnostic=diagnostic(t)))
            if t['crossings']:
                cohorts['live_across_values'] += 1
                cohorts['live_across_width_'+str(t['width'])] += 1
                cohorts['live_across_occurrences'] += len(t['crossings'])
            if tid in argument_ids:
                cohorts['argument_values'] += 1
                cohorts['argument_capture_write_code_bytes'] += capture_bytes
                cohorts['argument_values_with_capture_writes'] += bool(capture_bytes)
                if producer['kind']=='Call':
                    cohorts['result_and_argument_values'] += 1
                    cohorts['result_and_argument_capture_write_code_bytes'] += capture_bytes
                if t['home'] and len(t['uses']) == 1 and t['uses'][0]['kind']=='Call':
                    cohorts['materialized_sole_call_argument_values'] += 1
                    cohorts['materialized_sole_call_argument_width_'+str(t['width'])] += 1
                    cohorts['sole_call_argument_capture_write_code_bytes'] += capture_bytes
                    cohorts['sole_call_argument_values_with_capture_writes'] += bool(capture_bytes)
                    if t['uses'][0]['block'] == producer['block'] and t['uses'][0]['index'] == producer.get('index',-2)+1:
                        cohorts['materialized_adjacent_sole_call_argument_values'] += 1
                        cohorts['adjacent_sole_call_argument_values_with_capture_writes'] += bool(capture_bytes)
                        cohorts['adjacent_sole_call_argument_capture_write_code_bytes'] += capture_bytes
        routine_bytes = 0
        for c in r['calls']:
            require(all(a['value']['kind'] != 'temp' or a['value']['id'] in temps for a in c['arguments']), 'Unknown argument temp')
            span = c['span']
            selected = []
            before = transfer = after = guard = 0
            shape = 'terminal_forward'
            if span is not None:
                require(0 <= span[0] <= span[1] <= p['size'], 'Call span outside routine')
                spans = [s for s in l['spans'] if (s['block'],s['index']) == (c['block'],c['index'])]
                require(len(spans)==1 and [spans[0]['start'],spans[0]['end']]==span and spans[0]['kind'].startswith('Call/'), 'Call span mismatch')
                selected = [e for e in r['instructions'] if e['start'] < span[1] and e['end'] > span[0]]
                transfers = [e for e in selected if e['control'] in ('call','forward')]
                require(len(transfers)==1 or span[0]==span[1], 'Call span has ambiguous transfer')
                if transfers:
                    t = transfers[0]
                    guard = sum(pc in guards for pc in range(p['address']+span[0], p['address']+span[1]))
                    transfer = t['end']-t['start']
                    before = t['start']-span[0]-guard
                    after = span[1]-t['end']
                    require(min(before,after)>=0 and before+transfer+after+guard==span[1]-span[0], 'Call phase accounting mismatch')
                    pre = [decoded[p['address']+e['start']][0][0] for e in selected if e['start'] < t['start'] and p['address']+e['start'] not in guards]
                    shape = ('terminal_forward' if t['control']=='forward' else 'indirect_reserved' if c['target_kind']=='indirect'
                             else 'pushes' if any(op in (0x48,0xf4) for op in pre) else 'reserved_stores')
                else:
                    shape = 'fused_with_producer'
            result = temps[c['result']['id']] if c['result'] else None
            adjacent_return = bool(result and len(result['uses'])==1 and result['uses'][0]['kind']=='Return'
                and result['uses'][0]['block']==c['block'] and result['uses'][0]['index']==c['index']+1)
            cohorts['logical_calls'] += 1
            cohorts['call_shape_'+shape] += 1
            cohorts['call_target_'+c['target_kind']] += 1
            cohorts['calls_in_cyclic_blocks'] += c['cyclic']
            cohorts['calls_with_live_values'] += bool(c['live_across'])
            cohorts['zero_argument_calls'] += not c['arguments']
            cohorts['outgoing_padding_bytes'] += c['outgoing_bytes']-sum(a['width'] for a in c['arguments'])
            if result:
                cohorts['logical_result_values'] += 1
                cohorts['result_capture_write_code_bytes'] += capture_by_temp[result['id']]
                cohorts['result_values_with_capture_writes'] += bool(capture_by_temp[result['id']])
                cohorts['result_home_'+((result['home'] or {}).get('kind','none'))] += 1
                cohorts['result_adjacent_return'] += adjacent_return
                cohorts['materialized_result_adjacent_return'] += bool(adjacent_return and result['home'])
                cohorts['adjacent_return_values_with_capture_writes'] += bool(adjacent_return and capture_by_temp[result['id']])
                if result['home'] and len(result['uses'])==1:
                    use = result['uses'][0]
                    cohorts['materialized_result_sole_use_'+use['kind']] += 1
                    if use['block']==c['block'] and use['index']==c['index']+1:
                        cohorts['materialized_result_adjacent_'+use['kind']] += 1
                        cohorts['materialized_adjacent_result_width_'+str(result['width'])] += 1
                        cohorts['adjacent_result_capture_write_code_bytes'] += capture_by_temp[result['id']]
                        cohorts['adjacent_result_values_with_capture_writes'] += bool(capture_by_temp[result['id']])
                        cohorts['adjacent_result_'+use['kind']+'_capture_write_code_bytes'] += capture_by_temp[result['id']]
            else:
                cohorts['discarded_nonvoid_results' if c['declared_result_bytes'] else 'void_calls'] += 1
            non_guard = before+transfer+after
            routine_bytes += non_guard
            calls.append(dict(routine=r['name'],block=c['block'],index=c['index'],target_kind=c['target_kind'],target=c['target_name'] or str(c['target_id']),
                arguments=len(c['arguments']),argument_bytes=sum(a['width'] for a in c['arguments']),outgoing_bytes=c['outgoing_bytes'],
                shape=shape,pre_transfer_bytes=before,transfer_bytes=transfer,post_transfer_bytes=after,guard_bytes=guard,non_guard_bytes=non_guard,
                result_bytes=c['declared_result_bytes'],result_home=(result['home'] or {}).get('kind','none') if result else 'discarded',
                result_capture_write_code_bytes=capture_by_temp[result['id']] if result else 0,adjacent_return=adjacent_return,
                live_across_values=len(c['live_across']),cyclic=c['cyclic']))
        routines.append(dict(routine=r['name'],code_bytes=p['size'],calls=len(r['calls']),call_non_guard_bytes=routine_bytes,
            frame=r['frame'],spill=r['spill'],local_peak=r['local_peak'],live_across_values=sum(bool(t['crossings']) for t in temps.values())))
        if r['helper']:
            helpers.append(dict(routine=r['name'],width=r['helper']['width'],operation=r['helper']['operation'],signed=r['helper']['signed'],body_bytes=p['size']))
    require(cohorts['logical_calls']==sum(transfer_count.values()), 'Logical calls and physical transfers disagree')
    helper_by_name = {h['routine']:h for h in helpers}
    for h in helpers:
        sites = [c for c in calls if c['target_kind']=='helper' and c['target']==h['routine']]
        h['call_sites'] = len(sites)
        h['caller_non_guard_bytes'] = sum(c['non_guard_bytes'] for c in sites)
    require(all(c['target'] in helper_by_name for c in calls if c['target_kind']=='helper'), 'Unresolved helper identity')
    call_phases = {key:sum(c[key] for c in calls) for key in ('pre_transfer_bytes','transfer_bytes','post_transfer_bytes','guard_bytes','non_guard_bytes')}
    cohorts['call_routines'] = sum(r['calls']>0 for r in routines)
    cohorts['live_across_routines'] = sum(r['live_across_values']>0 for r in routines)
    placement_summary = {k:sum(r[k] for r in placement['routines']) for k in ('values','materialized','borrowed','register_intervals','component_intervals','mixed_homes','loop_homes','call_segments','transfers')}
    placement_summary.update(call_segment_routines=sum(r['call_segments']>0 for r in placement['routines']), routines=len(placement['routines']),opaque_routines=len(placement['opaque_routines']))
    return dict(image=summary,cohorts=dict(cohorts),call_phases=call_phases,physical_transfers=dict(transfer_count),
                static_nontransfer_access_bytes=dict(traffic),diagnostic_cohorts=dict(diagnostic_counts),
                observed_forwarding_requests=dict(request_counts),placement=placement_summary), calls, routines, helpers, captures


def report(output, destination, check=False):
    provenance = read(output/'provenance.json')
    provenance['report_tools'] = {str(p.relative_to(ROOT)):digest(p) for p in (
        Path(__file__),Path(__file__).with_name('test_exec_call_audit.py'),
        Path(__file__).with_name('exec_record_baseline.py'),Path(__file__).parent.parent/'disassemble65816.py')}
    stage7 = read(STAGE7)
    profiles, call_rows, routine_rows, helper_rows, capture_rows = {}, [], [], [], []
    examples = None
    for profile in PROFILES:
        directory = output/profile
        for name in FILES:
            require(digest(directory/name)==provenance['profiles'][profile]['artifacts'][name], 'Audit input hash mismatch: '+profile+'/'+name)
        require(digest(directory/'probe.image.json')==stage7['profiles'][profile]['artifact_hashes']['probe.image.json'], 'Stage-7 image identity mismatch')
        result,*tables = audit_profile(*(read(directory/name) for name in FILES[:2]),read(directory/FILES[3]),read(directory/FILES[2]))
        if profile=='optimized-release':
            examples = example_listing(read(directory/FILES[0]),read(directory/FILES[1]),read(directory/FILES[3]),tables[3])
        profiles[profile] = result
        for table, rows in zip((call_rows,routine_rows,helper_rows,capture_rows), tables):
            table.extend(dict(profile=profile,**row) for row in rows)
    result = dict(schema=1,compiler_revision=provenance['compiler_revision'],exec_revision=provenance['exec_revision'],
        rust=provenance['rust'],frozen_inputs_sha256=provenance['frozen_inputs_sha256'],profiles=profiles,
        scope='Static call/placement audit of the frozen kernel. Compiler code, not linked assembly, commands or whole-application runtime.',
        caveats=['Footprints are existing bytes, not removable bytes.', 'Static access bytes exclude ABI call/return summaries and stack guards; they are not executed VM traffic.',
                 'Diagnostic cohorts are ordered census classifications, not exhaustive internal allocator refusal reasons.',
                 'Canonical homes can coexist with register/DP caches or unused reservations after call-return forwarding.',
                 'Capture footprint counts actual store instruction bytes into the canonical temp home, not surrounding reloads or mode changes.',
                 'All three compiler images match stage 7 exactly. Frozen acceptance targets remain unchanged.'])
    artifacts = {'results.json':json.dumps(result,indent=2)+'\n','calls.csv':csv_text(call_rows),'routines.csv':csv_text(routine_rows),
                 'helpers.csv':csv_text(helper_rows),'captures.csv':csv_text(capture_rows),
                 'examples.lst':examples,
                 'provenance.json':json.dumps(provenance,indent=2)+'\n'}
    artifacts = {name+'.gz' if name in ('calls.csv','routines.csv','captures.csv') else name:
                 gzip.compress(text.encode(),mtime=0) if name in ('calls.csv','routines.csv','captures.csv') else text.encode()
                 for name,text in artifacts.items()}
    artifacts['evidence-sha256.json'] = (json.dumps({name:hashlib.sha256(data).hexdigest()
        for name,data in artifacts.items()},indent=2)+'\n').encode()
    for name,data in artifacts.items():
        path = destination/name
        if check:
            require(path.is_file() and path.read_bytes()==data, 'Published audit differs: '+name)
        else:
            destination.mkdir(parents=True,exist_ok=True)
            path.write_bytes(data)
    print('Audit verified' if check else 'Audit written', destination)
    print(json.dumps(profiles['optimized-release']['call_phases']))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation',choices=('collect','report'))
    parser.add_argument('--base',type=Path,default=ROOT/'target/record-placement-stage0')
    parser.add_argument('--output',type=Path,default=ROOT/'target/exec-call-audit')
    parser.add_argument('--destination',type=Path,default=ROOT/'docs/benchmarks/65816-exec-call-audit')
    parser.add_argument('--check',action='store_true')
    args = parser.parse_args()
    if args.operation == 'collect':
        require(not args.check, '--check applies to report')
        collect(args.base.resolve(),args.output.resolve())
    else:
        report(args.output.resolve(),args.destination.resolve(),args.check)


if __name__ == '__main__':
    main()
