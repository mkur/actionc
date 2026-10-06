#!/usr/bin/env python3
"""Qualify byte-identical foundation changes against the frozen Exec816 workload."""
import argparse
import csv
import hashlib
import json
from pathlib import Path
import subprocess
import sys

sys.dont_write_bytecode = True
from exec_record_baseline import PROFILES, digest, read, save, verify_inputs

ROOT = Path(__file__).resolve().parents[2]


def probe(base, output, binary):
    verify_inputs(base)
    for profile, (opt, guards) in PROFILES.items():
        destination = output / profile
        destination.mkdir(parents=True, exist_ok=True)
        subprocess.run([str(binary), str(base / 'profiles' / profile), str(base / 'exec'),
            str(destination / 'probe'), str(opt).lower(), str(guards).lower(), str(base / 'compiler')], check=True)
        for suffix in ('image.json', 'inventory.json'):
            before = base / 'profiles' / profile / ('probe.' + suffix)
            after = destination / ('probe.' + suffix)
            if before.read_bytes() != after.read_bytes():
                raise ValueError(f'Changed frozen {profile} {suffix}')
        print('Exact image, frame maps and inventory: ' + profile, flush=True)
    verify_inputs(base)


def host(base, output, binary, rounds):
    verify_inputs(base)
    commands = read(base / 'host-timing/results.json')['commands']
    manifest = output / 'host-manifest.json'
    save(manifest, dict(artifacts=[dict(compiler='actionc', case=p, mode=p, commands=[commands[p]],
        hashes={'image.json': digest(base / 'profiles' / p / 'probe.image.json')}) for p in PROFILES]))
    subprocess.run([sys.executable, '-B', str(Path(__file__).with_name('measure_host.py')),
        '--before', str(base / 'compiler/target/debug/actionc-65816'), '--after', str(binary),
        '--manifest', str(manifest), '--output', str(output / 'host-results.json'), '--rounds', str(rounds),
        '--expected-builds', '3'], check=True)
    result = read(output / 'host-results.json')
    # These pinned executables use optimized dev, matching stage 0, not Cargo
    # release. Per-child wait4, warm-up and interleaving are unchanged.
    result['methodology'] = result['methodology'].replace('Separate release CLI process', 'Separate optimized-dev CLI process')
    result['build_profile'] = dict(opt_level=3, debug=0, incremental=False)
    save(output / 'host-results.json', result)
    for row in result['per_build']:
        if row['wall_ratio'] > 1.05 or row['rss_ratio'] > 1.10:
            raise ValueError('Foundation overhead gate exceeded: ' + json.dumps(row))
    verify_inputs(base)


def publish(base, output, destination, stage=1):
    verify_inputs(base)
    destination.mkdir(parents=True, exist_ok=True)
    representatives = {}
    with (ROOT / 'docs/benchmarks/65816-record-placement-stage0/representatives.csv').open(newline='') as f:
        representatives.update((row['emitted_name'], row['name']) for row in csv.DictReader(f))
    result = dict(schema=1, baseline_revision=read(base / 'inputs.json')['compiler']['revision'], profiles={})
    rows, rep_rows = [], []
    placement_rows, placement_representatives = [], []
    for profile in PROFILES:
        directory = output / profile
        analysis = read(directory / 'probe.analysis.json')
        for suffix in ('image.json', 'inventory.json'):
            if digest(directory / ('probe.' + suffix)) != digest(base / 'profiles' / profile / ('probe.' + suffix)):
                raise ValueError('Changed frozen artifacts: ' + profile)
        facts = analysis['routines']
        summary = dict(analyzed_routines=len(facts), opaque_routines=len(analysis['opaque_routines']),
            analysis_seconds=sum(r['analysis_seconds'] for r in facts),
            image_sha256=digest(directory / 'probe.image.json'), inventory_sha256=digest(directory / 'probe.inventory.json'),
            analysis_sha256=digest(directory / 'probe.analysis.json'))
        for key in ('values', 'uses', 'checked_reachable_uses', 'captures', 'blocks', 'reachable_blocks', 'cyclic_blocks', 'edges', 'edge_arguments', 'solver_evaluations'):
            summary[key] = sum(r[key] for r in facts)
        result['profiles'][profile] = summary
        found = set()
        for r in facts:
            row = dict(profile=profile, **{k: v for k,v in r.items() if k != 'storage'})
            rows.append(row)
            if r['name'] in representatives:
                rep_rows.append(dict(source_routine=representatives[r['name']], **row))
                found.add(r['name'])
        if found != set(representatives):
            raise ValueError('Missing representatives: ' + repr(set(representatives) - found))
        if stage == 2:
            placement = read(directory / 'probe.placement.json')
            found = set()
            summary['placement'] = dict(routines=len(placement['routines']), opaque_routines=placement['opaque_routines'],
                sha256=digest(directory / 'probe.placement.json'))
            for r in placement['routines']:
                row = dict(profile=profile, **r)
                placement_rows.append(row)
                if r['name'] in representatives:
                    placement_representatives.append(dict(source_routine=representatives[r['name']], **row))
                    found.add(r['name'])
            if found != set(representatives):
                raise ValueError('Missing placement representatives: ' + repr(set(representatives) - found))
            for key in ('values','materialized','borrowed','register_intervals','component_intervals','redirected_locals',
                        'deferred_assignments','windows','record_windows','scalar_windows','address_windows','barriers',
                        'boundaries','edges','transfers','staging_bytes','x_mirror'):
                summary['placement'][key] = sum(r[key] for r in placement['routines'])
        native_before = read(base / 'native-vectors' / (profile + '.measurements.json'))
        native_after = read(output / (profile + '.measurements.json'))
        if native_before['measurements'] != native_after['measurements'] or native_before['control'] != native_after['control']:
            raise ValueError('Native output/cost changed: ' + profile)
        summary['native_vectors'] = len(native_after['measurements'])
        summary['native_executions'] = 2 * len(native_after['measurements'])
        summary['native_measurements_sha256'] = digest(output / (profile + '.measurements.json'))
    host = read(output / 'host-results.json')
    for row in host['per_build']:
        if row['wall_ratio'] > 1.05 or row['rss_ratio'] > 1.10:
            raise ValueError('Foundation overhead gate exceeded')
    result['host'] = host
    result['probe_binary_sha256'] = digest(base / 'rust-target/debug/actionc-exec-record-probe')
    result['frozen_inputs_sha256'] = digest(base / 'inputs.json')
    import re
    def checked_summaries(paths):
        rows = []
        for path in paths:
            rows.extend(re.findall(r'test result: ok\. (\d+) passed; 0 failed; (\d+) ignored', path.read_text()))
        if not rows:
            raise ValueError('Missing successful test summaries')
        return dict(targets=len(rows), passed=sum(int(a) for a,b in rows), ignored=sum(int(b) for a,b in rows))
    integration_logs = [output / 'integration.log']
    if 'test result: FAILED.' in integration_logs[0].read_text():
        integration_logs.append(output / 'integration-remaining.log')
    result['validation'] = dict(unit=checked_summaries([output / 'unit-final.log']),
        logical_graphs=checked_summaries([output / 'analysis-final.log']),
        integration=checked_summaries(integration_logs),
        native=checked_summaries([output / 'native-backend.native.log']),
        immutable_borrow=checked_summaries([output / 'borrow-check.log']),
        native_masks=['I=0', 'I=4'], qualification_manifests={})
    if stage == 2:
        result['stage'] = 2
        result['validation']['placement_graphs'] = result['validation'].pop('logical_graphs')
    import gzip
    import re
    for label in ['native-backend', *PROFILES]:
        log = output / (label + '.native.log')
        matches = re.findall(r'Qualification manifest: (.*)', log.read_text())
        if len(matches) != 1:
            raise ValueError('Missing successful CPU qualification manifest: ' + label)
        path = Path(matches[0])
        qualification = read(path)
        result['validation']['qualification_manifests'][label] = dict(sha256=digest(path),
            vm_base=qualification['vm_base'], vm_patch_sha256=qualification['vm_patch_sha256'])
        (destination / (label + '.qualification.json.gz')).write_bytes(gzip.compress(path.read_bytes(), mtime=0))
    result['validation']['logs'] = {p.name: digest(p) for p in output.glob('*.log')}
    source_paths = [*ROOT.joinpath('src').rglob('*.rs'), ROOT/'Cargo.toml', ROOT/'Cargo.lock',
        ROOT/'tools/compare65816/record_probe/Cargo.toml', ROOT/'tools/compare65816/record_probe/record_flow.act',
        ROOT/'tests/mir65816_logical_analysis.rs', ROOT/'tools/compare65816/measure_host.py', *ROOT.joinpath('tools/compare65816/record_probe/src').glob('*.rs'), Path(__file__)]
    source_hashes = {str(p.relative_to(ROOT)): digest(p) for p in source_paths}
    serialized = (json.dumps(source_hashes, sort_keys=True, separators=(',', ':')) + '\n').encode()
    result['compiler_inputs_sha256'] = hashlib.sha256(serialized).hexdigest()
    (destination / 'compiler-inputs.json').write_bytes(serialized)
    publications = [('routines.csv',rows),('representatives.csv',rep_rows)]
    if stage == 2:
        publications += [('placement.csv', placement_rows), ('placement-representatives.csv', placement_representatives)]
    for name, contents in publications:
        with (destination / name).open('w',newline='') as f:
            writer=csv.DictWriter(f,fieldnames=list(contents[0]),lineterminator='\n')
            writer.writeheader();writer.writerows(contents)
    save(destination / 'results.json',result)
    save(destination / 'evidence-sha256.json',{p.name:digest(p) for p in destination.iterdir()
        if p.is_file() and p.name not in ('README.md','evidence-sha256.json')})
    print('Published', destination)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation',choices=('probe','host','publish'))
    parser.add_argument('--base',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--binary',type=Path)
    parser.add_argument('--destination',type=Path)
    parser.add_argument('--rounds',type=int,default=5)
    parser.add_argument('--stage',type=int,choices=(1,2),default=1)
    args=parser.parse_args()
    base,output=args.base.resolve(),args.output.resolve()
    output.mkdir(parents=True,exist_ok=True)
    if args.operation in ('probe','host') and not args.binary:
        parser.error('--binary required')
    if args.operation=='probe':probe(base,output,args.binary.resolve())
    elif args.operation=='host':host(base,output,args.binary.resolve(),args.rounds)
    else:
        destination=args.destination or ROOT/f'docs/benchmarks/65816-record-placement-stage{args.stage}'
        publish(base,output,destination.resolve(),args.stage)


if __name__=='__main__':main()
