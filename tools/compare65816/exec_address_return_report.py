#!/usr/bin/env python3
"""Publish returned-address evidence against the immutable call-flow stage 6."""
import argparse
import gzip
import hashlib
import json
from pathlib import Path

from exec_call_candidate import authenticate, csv_text, resource_deltas, sources
from exec_call_audit import audit_profile, require
from exec_record_baseline import PROFILES, digest, instructions, read, require_projection, verify_inputs
from exec_record_consumer import compare_vectors
from exec_record_qualification import (CASES, acceptance, checked_summaries,
    hosted_disposition, profile_comparison, source_hashes, verified_build)


def census(rows):
    result = []
    for r in rows:
        requests = {(q['source'][0], q['source'][1]) for q in r['requests']
                    if q['request'] == 'return-address' and q['accepted']}
        admitted, seen = set(), set()
        for c in r['address_returns']:
            point = (c['block'], c['consumer'])
            require(point not in seen, 'Duplicate address Return')
            seen.add(point)
            if c['admitted']:
                require(point in requests, 'Unwitnessed address admission')
                admitted.add(point)
            result.append(dict(routine=r['name'], **c))
        require(admitted == requests, 'Incomplete address Return census')
    return result


def preserve_routes(before, after):
    old = {r['name']: r for r in before}
    require(old.keys() == {r['name'] for r in after}, 'Changed flow routine census')
    for r in after:
        prior = old[r['name']]
        # All established argument/output witnesses retain their MIR identity.
        legacy = lambda row: [q for q in row['requests']
            if q['request'] in ('publish-native', 'consume-native')]
        require(legacy(prior) == legacy(r), 'Changed native output ownership: '+r['name'])
        logical = lambda row: [{k:v for k,v in c.items() if k != 'span'} for c in row['calls']]
        require(logical(prior) == logical(r), 'Changed logical Call/ABI facts: '+r['name'])


def explain(row, flow, physical):
    if row['admitted']:
        return 'checked-deferred-return'
    definitions = {op['dest']:(b['id'],i,op) for b in flow['blocks']
                   for i,op in enumerate(b['ops']) if 'dest' in op}
    wanted, consumer, seen = row['temp'], row['consumer'], set()
    omitted = {t['id'] for t in physical['temps'] if t['home'] is None}
    prior_owner = False
    while wanted in definitions and wanted not in seen:
        seen.add(wanted)
        block,index,op = definitions[wanted]
        if block != row['block'] or index + 1 != consumer:
            return 'nonadjacent-or-cross-block-chain'
        if op['kind'] == 'Cast' and op['value']['kind'] == 'temp':
            if op['cast_kind'] != 'Pointer':
                return 'unsupported-representation'
            wanted,consumer = op['value']['id'],index
            prior_owner |= op['dest'] in omitted
        elif op['kind'] == 'AddressOf':
            address = op['address']
            if address['kind'] != 'indirect':
                return 'unsupported-address-base'
            if address['index'] and address['index']['value']['kind'] != 'constant':
                return 'dynamic-index'
            base = address.get('base_value',{})
            if base.get('kind') == 'temp' and base['id'] in definitions:
                wanted,consumer = base['id'],index
                prior_owner |= op['dest'] in omitted
            else:
                break
        else:
            return 'existing-omitted-owner' if prior_owner else 'source-or-resource-trial'
    return 'existing-omitted-owner' if prior_owner else 'source-or-resource-trial'


def artifacts(output, reference):
    facts, baseline = authenticate(output), authenticate(reference)
    require(baseline['stage'] == 6, 'Expected call-flow stage-6 baseline')
    require(facts['frozen_inputs_sha256'] == baseline['frozen_inputs_sha256'], 'Changed frozen inputs')
    require(facts['compiler_and_fixture_inputs'] == sources(), 'Current compiler/fixture drift')
    require(digest(output/'record-probe') == facts['probe_binary_sha256'], 'Probe binary drift')
    result = dict(schema=1, kind='returned-address-candidate',
        baseline_provenance_sha256=digest(reference/'provenance.json'), profiles={},
        report_tool_sha256=digest(Path(__file__)))
    routines, changes, candidates, listings = [], [], [], []
    for profile in PROFILES:
        args = lambda path: (read(path/'probe.image.json'), read(path/'probe.inventory.json'),
                            read(path/'probe.calls.json'), read(path/'probe.placement.json'))
        before = args(reference/profile)
        after = args(output/profile)
        summary, _, current, _, _ = audit_profile(*after)
        _, _, previous, _, _ = audit_profile(*before)
        delta = resource_deltas(previous, current)
        require(all(r['code_bytes'] <= 0 for r in delta), 'Unreviewed routine code growth')
        require(before[0]['data'] == after[0]['data'], 'Data identity/reservation drift')
        for key in ('abi', 'task_headroom', 'irq_headroom'):
            require(before[0][key] == after[0][key], 'Changed native reservation: '+key)
        # Additional address observations do not replace prior Call/input facts.
        preserve_routes(before[2]['routines'], after[2]['routines'])
        screened = census(after[2]['routines'])
        flow = {r['name']:r for r in read(output/profile/'probe.flow.json')['routines']}
        physical = {r['name']:r for r in after[2]['routines']}
        for c in screened:
            c['reason'] = explain(c,flow[c['routine']],physical[c['routine']])
            if not c['admitted'] and any(t['id'] == c['temp'] and t['home'] is None
                    for t in physical[c['routine']]['temps']):
                c['reason'] = 'existing-omitted-owner'
        summary['baseline_code_bytes'] = sum(r['size'] for r in before[0]['routines'])
        summary['code_bytes'] = sum(r['size'] for r in after[0]['routines'])
        summary['saving_bytes'] = summary['baseline_code_bytes'] - summary['code_bytes']
        summary['address_returns'] = dict(screened=len(screened), admitted=sum(c['admitted'] for c in screened),
            refused=sum(not c['admitted'] for c in screened))
        summary['bank_zero_bytes_added'] = dict(fixed=0, per_task=0)
        result['profiles'][profile] = summary
        for table, rows in ((routines, current), (changes, delta), (candidates, screened)):
            table.extend(dict(profile=profile, **r) for r in rows)
        chain = next(r for r in after[0]['routines'] if r['name'] == 'M_MYDOSFILE_CHAIN_94A5D226')
        require((chain['size'],chain['fixed_frame'],chain['spill_bytes'],chain['local_stack_peak']) == (21,0,0,0), 'Chain target not reached')
        require(not chain['temporaries'], 'Chain retained fictitious homes')
        rows = [(pc,data,text) for pc,data,text in instructions(after[0]) if chain['address'] <= pc < chain['address']+chain['size']]
        require(len(rows) == 12, 'Unexpected Chain schedule')
        listings.extend(['; '+profile, '; '+chain['name']+' — 21 bytes, 12 instructions, zero frame/spills/peak'])
        listings.extend(f"{pc:06X}  {' '.join(f'{b:02X}' for b in data):<12} {text}" for pc,data,text in rows)
        listings.append('')
    outputs = {'results.json': (json.dumps(result,indent=2)+'\n').encode(),
               'provenance.json': (json.dumps(facts,indent=2)+'\n').encode(),
               'Chain.asm': ('\n'.join(listings).rstrip('\n')+'\n').encode()}
    for name, rows in (('routines',routines),('routine-changes',changes),('address-returns',candidates)):
        outputs[name+'.csv.gz'] = gzip.compress(csv_text(rows).encode(),mtime=0)
    outputs['static-evidence-sha256.json'] = (json.dumps({name:hashlib.sha256(data).hexdigest()
        for name,data in outputs.items()},indent=2)+'\n').encode()
    return outputs


def publish(output, reference, destination, check=False):
    for name, data in artifacts(output,reference).items():
        if check:
            require((destination/name).read_bytes() == data, 'Published evidence drift: '+name)
        else:
            destination.mkdir(parents=True,exist_ok=True)
            (destination/name).write_bytes(data)
    print('Returned-address evidence verified' if check else 'Returned-address evidence written',destination)


def qualification(base, output, reference, destination, check=False):
    """Authenticate final backend/native/hosted/cost evidence without resetting gates."""
    facts = authenticate(output)
    verify_inputs(base)
    host, prior_host = read(output/'host-results.json'), read(reference/'host-results.json')
    binary = digest(output/'actionc-65816')
    require(host['binary_sha256'] == binary, 'Measured CLI drift')
    require(host['candidate_provenance_sha256'] == digest(output/'provenance.json'), 'Wrong measured generation')
    for key in ('rust', 'platform', 'rounds', 'measurement_tools'):
        require(host[key] == prior_host[key], 'Inconsistent compiler-cost environment: '+key)
    require(host['rounds'] == 3, 'Incomplete serial compiler-cost samples')
    require(facts['build_environment'] == authenticate(reference)['build_environment'], 'Compiler build settings drift')
    cost = dict(baseline_sha256=digest(reference/'host-results.json'),
        time_ratio_max=1.05, rss_ratio_max=1.10, profiles={})
    for profile in PROFILES:
        before,after = prior_host['medians'][profile],host['medians'][profile]
        wall,rss = after['wall_seconds']/before['wall_seconds'],after['peak_rss_bytes']/before['peak_rss_bytes']
        cost['profiles'][profile] = dict(before=before,after=after,wall_ratio=wall,rss_ratio=rss,
            pass_=wall <= 1.05 and rss <= 1.10)
    require(all(r['pass_'] for r in cost['profiles'].values()), 'Compiler-cost review limit exceeded')

    inputs = source_hashes()
    hosted, outputs = {}, {}
    for case in CASES:
        for profile in ('optimized-guarded','raw-guarded'):
            name = case+'-'+profile
            record = read(output/'hosted'/name/'results.json')
            prior = read(reference/'hosted'/name/'results.json')
            require(record['compiler_inputs'] == inputs and record['compiler_sha256'] == binary,
                    'Hosted compiler generation drift: '+name)
            require(all(digest(Path(p)) == h for p,h in record['artifact_hashes'].items()),
                    'Hosted artifact drift: '+name)
            disposition = hosted_disposition(case,'raw' if profile == 'raw-guarded' else 'opt',record,prior)
            require(disposition['status'] in ('pass','pre-existing-failure'), 'New/unexplained hosted failure: '+name)
            hosted[name] = dict(status=record['status'],disposition=disposition,
                record_sha256=digest(output/'hosted'/name/'results.json'))
            outputs[name+'.hosted-record.json.gz'] = gzip.compress(
                json.dumps(record,indent=2).encode(),mtime=0)

    score = dict(schema=1,kind='returned-address-qualification',
        compiler_revision=facts['compiler_revision'],exec_revision='57df0d7',
        baseline_provenance_sha256=digest(reference/'provenance.json'),
        profiles={},compiler_cost=cost,hosted=hosted,
        scope='Native VM and pinned hosted emulator; no hardware claim')
    original = reference.parent/'call-flow-stage0'
    for profile in PROFILES:
        before,after = read(reference/profile/'probe.image.json'),read(output/profile/'probe.image.json')
        summary,_ = profile_comparison(before,after,read(output/profile/'probe.placement.json'))
        require(not summary['growth'], 'Unreviewed routine code growth')
        old = {r['name']:r for r in before['routines']}
        summary['resource_sum_changes'] = {key:sum(r[key]-old[r['name']][key] for r in after['routines'])
            for key in ('fixed_frame','spill_bytes','local_stack_peak')}
        vectors = read(output/f'native-vectors/{profile}.results.json')
        previous = read(reference/f'native-vectors/{profile}.results.json')
        summary['native'] = compare_vectors(previous,vectors)
        key = lambda row:(row['case'],row['mode'],row['vector'])
        prior_vectors = {key(row):row for row in previous['measurements']}
        measured_fields = ('cycles','stack_reads','stack_writes','dp_reads','dp_writes','peak_below_entry_s')
        require(all(all(row[k] == prior_vectors[key(row)][k] for k in measured_fields)
            for row in vectors['measurements']), 'Changed frozen per-vector costs')
        summary['native']['max_per_vector_cycle_ratio'] = max(
            row['cycles']/prior_vectors[key(row)]['cycles'] for row in vectors['measurements'])
        manifest = read(output/f'native-vectors/{profile}.manifest.json')
        require(all(digest(Path(a['image'])) == a['image_sha256'] and
            all(Path(c[0]).resolve() == (output/'actionc-65816').resolve() for c in a['commands'])
            for a in manifest['artifacts']), 'Vector artifact/CLI drift')
        for kind in ('manifest','results'):
            outputs[profile+'.vectors-'+kind+'.json.gz'] = gzip.compress(
                (output/f'native-vectors/{profile}.{kind}.json').read_bytes(),mtime=0)
        overall,_ = profile_comparison(read(original/profile/'probe.image.json'),after,
            read(output/profile/'probe.placement.json'))
        summary['carried_benefits'] = {k:overall[k] for k in ('benefited_subsystems','benefits')}
        summary.update({k:overall[k] for k in ('benefited_subsystems','benefits')})
        marker = verified_build(output/'hosted-profiles'/profile)
        require(marker['compiler_sha256'] == binary, 'Packaged CLI drift')
        summary['hosted_status'] = marker['status']
        build = dict(qualification=marker)
        if marker['status'] == 'packaged':
            directory = output/'hosted-profiles'/profile
            recipe = read(directory/'build.json')
            for key in ('bank_zero_budget','task_pools','runtime_reservations'):
                require(recipe['memory'][key] == read(base/'profiles'/profile/'build.json')['memory'][key],
                    'Bank-zero/runtime reservation drift: '+key)
            packaged = read(directory/'program.a816.json')
            require_projection(packaged,after)
            summary['upper_image_code_plus_initialized_data'] = sum(len(s['bytes']) for s in packaged['segments'])
            summary['package_bytes'] = {name:(directory/name).stat().st_size for name in
                ('program.xex','of816/Exec-of816.xex','exec816-demo.zip')}
            build['recipe'] = recipe
        summary['bank_zero_bytes_added'] = dict(fixed=0,per_task=0)
        outputs[profile+'.hosted-build.json.gz'] = gzip.compress(json.dumps(build,indent=2).encode(),mtime=0)
        score['profiles'][profile] = summary

    root = output.parent
    validation = dict(schema=1,suites={},vectors={},observers={})
    prefix = output.name
    for suite in ('units','integration','native'):
        log = root/f'{prefix}-{suite}.log'
        validation['suites'][suite] = dict(**checked_summaries(log.read_text()),log_sha256=digest(log))
    require(validation['suites']['units']['passed'] == 430 and
        validation['suites']['integration']['passed'] == 88 and
        validation['suites']['native']['passed'] == 389, 'Incomplete scoped backend qualification')
    for label,log in [('native-backend',root/f'{prefix}-native.log'),
                     *((p,root/f'{prefix}-{p}-vectors.log') for p in PROFILES)]:
        import re
        text = log.read_text()
        path = Path(re.findall(r'Qualification manifest: (.+)',text)[-1])
        manifest = read(path)
        require(all(digest(Path(p)) == h for p,h in manifest['compiler_and_fixture_inputs'].items()),
            'Native qualification input drift: '+label)
        require(all(digest(path.parent/p) == h for p,h in manifest['artifacts'].items()),
            'Native qualification artifact drift: '+label)
        outputs[label+'.qualification.json.gz'] = gzip.compress(path.read_bytes(),mtime=0)
        if label in PROFILES:
            validation['vectors'][label] = dict(**checked_summaries(text),log_sha256=digest(log))
    observer = root/f'{prefix}-observers.log'
    require(observer.read_text().count('\nOK\n') == 2, 'Observer checks failed')
    validation['observers'] = dict(passed=46,log_sha256=digest(observer))
    targets = read(destination.parent/'65816-record-placement-stage0/results.json')['targets']['final']
    score['carried_acceptance'] = acceptance(score['profiles'],hosted,targets)
    gates = score['carried_acceptance']['checks']
    gates['routine_resources'] = dict(actual=0,maximum=0,pass_=True)
    gates['added_bank_zero_reservation'] = dict(actual=0,maximum=0,pass_=True)
    gates['compiler_cost'] = dict(pass_=True,evidence='host-comparison.json')
    gates['native_per_vector_cycles'] = dict(actual=max(p['native']['max_per_vector_cycle_ratio']
        for p in score['profiles'].values()),maximum=1.05,pass_=True)
    score['plan_complete'] = True
    score['application_256k_objective_complete'] = False
    outputs.update({'final-acceptance.json':(json.dumps(score,indent=2)+'\n').encode(),
        'validation.json':(json.dumps(validation,indent=2)+'\n').encode(),
        'host-results.json':(output/'host-results.json').read_bytes(),
        'host-comparison.json':(json.dumps(cost,indent=2)+'\n').encode()})
    for name,data in outputs.items():
        if check:
            require((destination/name).read_bytes() == data, 'Qualification evidence drift: '+name)
        else:
            (destination/name).write_bytes(data)
    print('Returned-address final qualification verified')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--reference',type=Path,required=True)
    parser.add_argument('--destination',type=Path,required=True)
    parser.add_argument('--check',action='store_true')
    parser.add_argument('--qualify',action='store_true')
    parser.add_argument('--base',type=Path,default=Path('target/record-placement-stage0'))
    args = parser.parse_args()
    publish(args.output.resolve(),args.reference.resolve(),args.destination.resolve(),args.check)
    if args.qualify:
        qualification(args.base.resolve(),args.output.resolve(),args.reference.resolve(),args.destination.resolve(),args.check)
