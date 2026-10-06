#!/usr/bin/env python3
"""Qualify placement consumers on immutable stage-0 inputs and independent CPU traces."""
import argparse
import csv
import gzip
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

sys.dont_write_bytecode = True
from exec_record_baseline import PROFILES, digest, read, save, verify_inputs
from exec_record_report import REPRESENTATIVES

ROOT = Path(__file__).resolve().parents[2]


def probe(base, output, binary):
    verify_inputs(base)
    for profile, (opt, guards) in PROFILES.items():
        destination = output / profile
        destination.mkdir(parents=True, exist_ok=True)
        subprocess.run([str(binary), str(base / 'profiles' / profile), str(base / 'exec'),
            str(destination / 'probe'), str(opt).lower(), str(guards).lower(), str(base / 'compiler')], check=True)
    verify_inputs(base)


def host(base, output, binary, rounds):
    verify_inputs(base)
    commands = read(base / 'host-timing/results.json')['commands']
    manifest = output / 'host-manifest.json'
    save(manifest, dict(artifacts=[dict(compiler='actionc', case=p, mode=p, commands=[commands[p]],
        hashes={'image.json': digest(base / 'profiles' / p / 'probe.image.json')},
        compiler_image_hashes=dict(before=digest(base / 'profiles' / p / 'probe.image.json'),
            after=digest(output / p / 'probe.image.json'))) for p in PROFILES]))
    subprocess.run([sys.executable, '-B', str(Path(__file__).with_name('measure_host.py')),
        '--before', str(base / 'compiler/target/debug/actionc-65816'), '--after', str(binary),
        '--manifest', str(manifest), '--output', str(output / 'host-results.json'), '--rounds', str(rounds),
        '--expected-builds', '3'], check=True)
    result = read(output / 'host-results.json')
    result['methodology'] = result['methodology'].replace('Separate release CLI process', 'Separate optimized-dev CLI process')
    result['build_profile'] = dict(opt_level=3, debug=0, incremental=False)
    save(output / 'host-results.json',result)
    verify_inputs(base)


def references(base, output):
    verify_inputs(base)
    for profile in PROFILES:
        manifest = read(base / 'native-vectors' / f'{profile}.manifest.json')
        for artifact in manifest['artifacts']:
            if digest(Path(artifact['image'])) != artifact['image_sha256']:
                raise ValueError('Changed archived native image')
        manifest.update(artifact_verification='archived', observe_external_accesses=True)
        save(output / f'{profile}.reference.manifest.json', manifest)


def compare_vectors(before, after):
    rows = before['measurements'], after['measurements']
    keys = lambda r: (r['case'], r['mode'], r['vector'])
    left, right = ({keys(r): r for r in table} for table in rows)
    if len(left) != len(rows[0]) or len(right) != len(rows[1]):
        raise ValueError('Duplicate native vector measurement')
    if left.keys() != right.keys():
        raise ValueError('Changed native vector census')
    if before['manifest']['cases'] != after['manifest']['cases']:
        raise ValueError('Changed independent vector inputs/oracles')
    totals = {side: dict(cycles=0, private=0, peak=0) for side in ('before', 'after')}
    for key, a in left.items():
        b = right[key]
        if not a['correct'] or not b['correct'] or a['result'] != b['result'] or a['args'] != b['args']:
            raise ValueError('Native oracle failure: ' + repr(key))
        if a.get('external_accesses') is None or a['external_accesses'] != b.get('external_accesses'):
            raise ValueError('Changed external access trace: ' + repr(key))
        if b['cycles'] > a['cycles'] * 1.05 or b['peak_below_entry_s'] > a['peak_below_entry_s']:
            raise ValueError('Native cost/stack regression: ' + repr(key))
        for side, row in [('before', a), ('after', b)]:
            totals[side]['cycles'] += row['cycles']
            totals[side]['private'] += sum(row[k] for k in ('stack_reads', 'stack_writes', 'dp_reads', 'dp_writes'))
            totals[side]['peak'] = max(totals[side]['peak'], row['peak_below_entry_s'])
    totals['vectors'] = len(left)
    totals['independent_probe'] = {side: {
        'cycles': sum(r['cycles'] for r in table if r['case'] == 'Flow'),
        'private': sum(sum(r[k] for k in ('stack_reads', 'stack_writes', 'dp_reads', 'dp_writes')) for r in table if r['case'] == 'Flow')}
        for side, table in zip(('before','after'), rows)}
    return totals


def score(base, output):
    verify_inputs(base)
    result = dict(schema=1, stage=3, baseline_revision=read(base / 'inputs.json')['compiler']['revision'],
        exec_revision=read(base / 'inputs.json')['exec']['revision'], profiles={})
    rows = []
    for profile in PROFILES:
        before = read(base / 'profiles' / profile / 'probe.image.json')
        after = read(output / profile / 'probe.image.json')
        old, new = ({r['name']:r for r in image['routines']} for image in (before, after))
        for field in ('abi','task_headroom','irq_headroom'):
            if before[field] != after[field]: raise ValueError('Changed ABI/reservation: ' + field)
        data = lambda image: sorted((r['id'], r['address'], r['size']) for r in image['data'])
        if data(before) != data(after): raise ValueError('Changed data/bank-zero reservation')
        if old.keys() != new.keys():
            raise ValueError('Changed routine census')
        for name in old:
            for field in ('fixed_frame', 'local_stack_peak', 'spill_bytes'):
                if new[name][field] > old[name][field]:
                    raise ValueError(f'Frame/peak regression {profile}/{name}/{field}: {old[name][field]} -> {new[name][field]}')
        summary = dict(before_bytes=sum(r['size'] for r in old.values()), after_bytes=sum(r['size'] for r in new.values()),
            image_sha256=digest(output / profile / 'probe.image.json'), inventory_sha256=digest(output / profile / 'probe.inventory.json'))
        summary['representatives'] = {}
        for name in REPRESENTATIVES:
            emitted = next(n for n in old if n.startswith('M_' + name.replace('.', '_') + '_'))
            row = dict(profile=profile, name=name, before=old[emitted]['size'], after=new[emitted]['size'],
                frame_before=old[emitted]['fixed_frame'], frame_after=new[emitted]['fixed_frame'],
                peak_before=old[emitted]['local_stack_peak'], peak_after=new[emitted]['local_stack_peak'])
            rows.append(row); summary['representatives'][name] = row
        b = sum(r['before'] for r in summary['representatives'].values())
        a = sum(r['after'] for r in summary['representatives'].values())
        summary.update(representative_before=b, representative_after=a, representative_ratio=a/b)
        benefited = {name.split('.')[0] for name,r in summary['representatives'].items() if r['after'] < r['before']}
        if a > .98*b or len(benefited) < 2 or summary['after_bytes'] > summary['before_bytes']:
            raise ValueError('First benefit tranche missed: ' + json.dumps(summary))
        native_before = read(output / f'{profile}.reference.measurements.json')
        expected_reference = read(base / 'native-vectors' / f'{profile}.manifest.json')
        expected_reference.update(artifact_verification='archived', observe_external_accesses=True)
        if native_before['manifest'] != expected_reference: raise ValueError('Changed archived baseline manifest')
        for artifact in expected_reference['artifacts']:
            if digest(Path(artifact['image'])) != artifact['image_sha256']: raise ValueError('Changed archived image')
        native_after = read(output / f'{profile}.measurements.json')
        frozen = read(base / 'native-vectors' / f'{profile}.measurements.json')['measurements']
        # Archived execution must reproduce the original independent CPU costs.
        if len(frozen) != len(native_before['measurements']): raise ValueError('Archived baseline census drift')
        for x,y in zip(frozen, native_before['measurements']):
            for k in ('case','mode','vector','result','cycles','instructions','stack_reads','stack_writes','dp_reads','dp_writes','peak_below_entry_s'):
                if x[k] != y[k]: raise ValueError('Archived baseline drift: ' + k)
        summary['native'] = compare_vectors(native_before, native_after)
        if profile != 'raw-guarded' and summary['native']['after']['peak'] > 42:
            raise ValueError('Optimized native stack peak exceeds stage-0 gate')
        independent = summary['native']['independent_probe']
        if independent['after']['cycles'] >= independent['before']['cycles'] or independent['after']['private'] >= independent['before']['private']:
            raise ValueError('No independent probe benefit')
        summary['benefited_subsystems'] = sorted(benefited)
        summary['native_sha256'] = {p.name:digest(p) for p in output.glob(profile + '*.measurements.json')}
        placement = read(output / profile / 'probe.placement.json')['routines']
        summary['mixed_homes'] = sum(r['mixed_homes'] for r in placement)
        summary['backed_residences'] = sum(r['backed_residences'] for r in placement)
        result['profiles'][profile] = summary
    save(output / 'score.json', result)
    return result, rows


def publish(base, output, destination):
    result, rows = score(base, output)
    destination.mkdir(parents=True, exist_ok=True)
    result['validation'] = {}
    for label in ('unit-final', 'integration', 'native-backend'):
        log = output / (label + '.log')
        summaries = re.findall(r'test result: ok\. (\d+) passed; 0 failed; (\d+) ignored', log.read_text())
        if not summaries or 'test result: FAILED.' in log.read_text(): raise ValueError('Missing final successful tests: ' + label)
        result['validation'][label] = dict(targets=len(summaries), passed=sum(int(a) for a,b in summaries), ignored=sum(int(b) for a,b in summaries))
    for label in ('native-backend', *PROFILES, *(p + '.reference' for p in PROFILES)):
        log = output / (label + '.native.log') if label != 'native-backend' else output / 'native-backend.log'
        matches = re.findall(r'Qualification manifest: (.*)', log.read_text())
        if len(matches) != 1: raise ValueError('Missing successful qualification: ' + label)
        path = Path(matches[0])
        qualification = read(path)
        for name, expected in qualification['compiler_and_fixture_inputs'].items():
            if digest(ROOT / name) != expected: raise ValueError('Qualification belongs to a different source generation: ' + name)
        result['validation'][label + '-manifest'] = digest(path)
        (destination / (label + '.qualification.json.gz')).write_bytes(gzip.compress(path.read_bytes(), mtime=0))
    host = read(output / 'host-results.json')
    result['host'] = host
    followup = output / 'host-raw-followup.json'
    if followup.exists():
        raw = read(followup)
        if raw['builds'] != 1 or [r['case'] for r in raw['per_build']] != ['raw-guarded']:
            raise ValueError('Raw follow-up has a different workload')
        for name in ('before', 'after'):
            if raw['binaries'][name]['sha256'] != host['binaries'][name]['sha256']:
                raise ValueError('Raw follow-up has a different compiler')
            expected = next(s['image_sha256'] for s in host['samples'] if s['case'] == 'raw-guarded' and s['compiler'] == name)
            if any(s['image_sha256'] != expected for s in raw['samples'] if s['compiler'] == name):
                raise ValueError('Raw follow-up has a different image')
        result['host_raw_followup'] = raw
        result['host_raw_followup_sha256'] = digest(followup)
    result['frozen_inputs_sha256'] = digest(base / 'inputs.json')
    paths = [*ROOT.joinpath('src').rglob('*.rs'), *ROOT.joinpath('tools/native65816-runtime-tests/tests').rglob('*.rs'),
        ROOT/'Cargo.toml', ROOT/'Cargo.lock', *Path(__file__).parent.glob('exec_record_*.py'),
        Path(__file__).with_name('measure_host.py'), *Path(__file__).with_name('record_probe').joinpath('src').glob('*.rs'),
        Path(__file__).with_name('record_probe')/'Cargo.toml', Path(__file__).with_name('record_probe')/'record_flow.act',
        ROOT/'tests/mir65816_emission.rs', ROOT/'tests/mir65816_state_boundary.rs',
        ROOT/'tests/fixtures/mir65816-state-boundary.txt']
    hashes = {str(p.relative_to(ROOT)):digest(p) for p in paths}
    save(destination / 'compiler-inputs.json', hashes)
    result['compiler_inputs_sha256'] = digest(destination / 'compiler-inputs.json')
    with (destination / 'representatives.csv').open('w',newline='') as file:
        writer=csv.DictWriter(file,fieldnames=list(rows[0]),lineterminator='\n');writer.writeheader();writer.writerows(rows)
    save(destination / 'results.json',result)
    save(destination / 'evidence-sha256.json',{p.name:digest(p) for p in destination.iterdir() if p.name not in ('README.md','evidence-sha256.json')})


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation',choices=('probe','references','score','host','publish'))
    parser.add_argument('--base',type=Path,required=True);parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--rounds',type=int,default=5);parser.add_argument('--binary',type=Path);parser.add_argument('--destination',type=Path)
    args=parser.parse_args();base=args.base.resolve();output=args.output.resolve();output.mkdir(parents=True,exist_ok=True)
    if args.operation=='probe':
        if not args.binary: parser.error('--binary required')
        probe(base,output,args.binary.resolve())
    elif args.operation=='host':
        if not args.binary: parser.error('--binary required')
        host(base,output,args.binary.resolve(),args.rounds)
    elif args.operation=='references': references(base,output)
    elif args.operation=='score': score(base,output)
    else: publish(base,output,args.destination or ROOT/'docs/benchmarks/65816-record-placement-stage3')


if __name__=='__main__':main()
