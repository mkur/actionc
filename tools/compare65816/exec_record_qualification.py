#!/usr/bin/env python3
"""Artifact-bound final qualification of the frozen record/placement workload.

Acceptance failures are published and return a nonzero exit code. They do not
change the stage-0 numerical targets or authorize a different provider ABI.
"""
import argparse
import csv
import gzip
import json
import os
from pathlib import Path
import re
import subprocess
import sys

sys.dont_write_bytecode = True
from exec_record_baseline import (
    PROFILES, digest, inventory, read, require_projection, save, verify_inputs,
)
from exec_record_consumer import compare_vectors
from exec_record_frame_maps import install as install_frame_maps
from exec_record_report import REPRESENTATIVES

ROOT = Path(__file__).resolve().parents[2]
PUBLISHED_BASE = ROOT / 'docs/benchmarks/65816-record-placement-stage0'
CASES = ('lists', 'ports', 'dos-streams', 'dos-routing', 'cooked', 'demo', 'of816')


def source_hashes():
    paths = [*ROOT.joinpath('src').rglob('*.rs'), ROOT/'Cargo.toml', ROOT/'Cargo.lock',
             *ROOT.joinpath('runtime/65816').glob('*'),
             *Path(__file__).parent.glob('exec_record_*.py'),
             *Path(__file__).parent.glob('test_exec_record_*.py'),
             Path(__file__).with_name('measure_host.py'),
             *Path(__file__).with_name('record_probe').joinpath('src').glob('*.rs'),
             Path(__file__).with_name('record_probe')/'Cargo.toml',
             Path(__file__).with_name('record_probe')/'record_flow.act']
    return {str(p.relative_to(ROOT)): digest(p) for p in paths if p.is_file()}


def baseline_native(profile, measurements):
    with (PUBLISHED_BASE/'native-metrics.csv').open(newline='') as file:
        old = {(r['case'], int(r['vector'])): r for r in csv.DictReader(file) if r['profile'] == profile}
    new = {(r['case'], r['vector']): r for r in measurements['measurements']}
    if len(new) != len(measurements['measurements']) or new.keys() != old.keys():
        raise ValueError('Rebuilt native baseline census differs from its publication')
    for key, row in new.items():
        for field in ('cycles', 'instructions', 'stack_reads', 'stack_writes', 'dp_reads', 'dp_writes',
                      'stack_check_cycles', 'stack_check_instructions', 'peak_below_entry_s'):
            if row[field] != int(old[key][field]):
                raise ValueError('Rebuilt native baseline drift: '+repr(key)+'/'+field)
        if not row['correct'] or old[key]['correct'] != 'True':
            raise ValueError('Rebuilt native baseline oracle failure')


def hosted_disposition(case, mode, record, prior):
    if record['status'] == 'pass':
        return dict(status='pass', previously_failed=prior['status'] == 'fail')
    if prior['status'] == 'pass':
        return dict(status='new-failure', owner='unassigned', error=record['error'])
    message = record.get('error', '')
    diagnostic = record.get('diagnostic') or ''
    if case == 'lists' and message == 'Resident image payload in bank zero':
        return dict(status='pre-existing-failure', owner='Exec hosted fixture packaging',
                    requirement='Declare and initialize bank-zero diagnostic storage outside resident image payload')
    if case in ('dos-streams', 'dos-routing') and 'console-storage-action.inc' in diagnostic:
        return dict(status='pre-existing-failure', owner='Exec fixture generation',
                    requirement='Generate the console include dependencies required by standalone DOS composition')
    if case == 'ports' and mode == 'raw' and message == 'Native interrupt/COP vectors not restored':
        return dict(status='pre-existing-failure', owner='Exec platform restoration',
                    requirement='Restore all saved native interrupt/COP vectors before the raw queue fixture returns')
    return dict(status='unexplained-failure', owner='unassigned', error=message, diagnostic=diagnostic)


def pinned_toolchain(native, directory, binary):
    """Select an already-built measured CLI; keep the frozen ABI validation."""
    directory, binary = directory.resolve(), binary.resolve()
    if not binary.is_file():
        raise ValueError('Missing measured compiler CLI')
    pin = native.COMPILER_PIN
    from generate_native_abi import generate
    for path, content in generate(directory).items():
        if path.read_text().replace('\r\n', '\n') != content:
            raise ValueError('Stale native ABI consumer: ' + str(path))
    abi = read(directory / pin['abi_json'])
    baseline_abi = read(ROOT / pin['abi_json'])
    if abi != baseline_abi or abi['abi'] != pin['abi']:
        raise ValueError('Compiler candidate requires an explicit ABI migration')
    revision = subprocess.check_output(['git', '-C', str(directory), 'rev-parse', 'HEAD'], text=True).strip()
    changes = subprocess.check_output(['git', '-C', str(directory), 'status', '--porcelain'], text=True).splitlines()
    return dict(directory=directory, binary=binary, revision=revision, changes=changes,
                override=revision != pin['revision'] or bool(changes),
                compiler_contract=pin, image_format_version=pin['image_format_version'],
                binary_sha256=digest(binary), abi_sha256=digest(directory / pin['abi_json']),
                abi_assembly_sha256=digest(directory / pin['abi_assembly']),
                memory_runtime_inputs={name: digest(directory/'runtime/65816'/name)
                                       for name in ('a816memory.act', 'memory.s', 'memory.json')})


class BootPinRoot:
    """Project only the boot helper's compiler pin; preserve every Exec path."""
    def __init__(self, root, pin):
        self.root, self.pin = root, pin

    def __fspath__(self):
        return os.fspath(self.root)

    def __truediv__(self, path):
        return self.pin if path == 'toolchain/actionc.json' else self.root/path


def measured_boot(module, original, toolchain, output, exec_build, upstream):
    """Retain strict boot revision checks against the explicitly measured CLI."""
    program = module.read_build(exec_build.resolve())
    for field, expected in (('revision', toolchain['revision']),
                            ('binary_sha256', toolchain['binary_sha256']),
                            ('abi_sha256', toolchain['abi_sha256'])):
        if program['build'].get(field) != expected:
            raise ValueError('Boot package does not match measured compiler: '+field)
    root = module.ROOT
    frozen = read(root/'toolchain/actionc.json')
    if frozen != toolchain['compiler_contract']:
        raise ValueError('Boot compiler contract differs from frozen native contract')
    output.mkdir(parents=True, exist_ok=True)
    pin = output/'measured-actionc.json'
    save(pin, dict(frozen, revision=toolchain['revision']))
    module.ROOT = BootPinRoot(root, pin)
    try:
        record = original(output, exec_build, upstream)
    finally:
        module.ROOT = root
    record['measured_compiler'] = dict(revision=toolchain['revision'],
        binary_sha256=toolchain['binary_sha256'], abi_sha256=toolchain['abi_sha256'],
        frozen_pin_sha256=digest(root/'toolchain/actionc.json'), measured_pin_sha256=digest(pin))
    save(output/'of816.json', record)
    return record


def build(base, output, directory, binary):
    verify_inputs(base)
    inputs = source_hashes()
    sys.path.insert(0, str(base / 'exec/tools'))
    import native_program as native
    toolchain = pinned_toolchain(native, directory, binary)
    install_frame_maps()
    native.compiler = lambda *a, **kw: toolchain
    import build_demo
    import build_of816
    os.environ.update(CARGO_INCREMENTAL='0', CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_DEV_OPT_LEVEL='3')
    original, command = build_demo.build, build_demo.compile_command
    monitor = build_demo.build_monitor
    build_demo.build_monitor = lambda output, exec_build, upstream: measured_boot(
        build_of816, monitor, toolchain, output, exec_build, upstream)
    for profile, (optimize, guards) in PROFILES.items():
        destination = output / profile
        destination.mkdir(parents=True, exist_ok=True)
        marker = destination / 'qualification-build.json'
        marker.unlink(missing_ok=True)

        def compile_native(*args, **kwargs):
            kwargs.update(optimize=optimize, stack_checks=guards)
            return original(*args, **kwargs)

        def compile_command(*args, **kwargs):
            kwargs['optimize'] = optimize
            return command(*args, **kwargs)

        build_demo.build, build_demo.compile_command = compile_native, compile_command
        status, error = 'packaged', None
        try:
            build_demo.bundle(destination, directory)
        except ValueError as failure:
            if profile != 'optimized-release' or str(failure) != 'Hosted o65 requires a checked kernel':
                raise
            status, error = 'unsupported', str(failure)
        finally:
            build_demo.build, build_demo.compile_command = original, command
        if inputs != source_hashes() or digest(binary) != toolchain['binary_sha256']:
            raise ValueError('Measured compiler inputs changed during packaging')
        save(marker, dict(profile=profile, status=status, error=error,
                          compiler_sha256=toolchain['binary_sha256'], compiler_revision=toolchain['revision'],
                          compiler_inputs=inputs, frozen_inputs_sha256=digest(base/'inputs.json'),
                          optimize=optimize, stack_checks=guards,
                          artifacts={str(p.relative_to(destination)): digest(p)
                                     for p in destination.rglob('*') if p.is_file() and p != marker}))
        print(profile + ': ' + (error or 'packaged, including disk commands and OF816'), flush=True)
    verify_inputs(base)


def probe(base, output, binary):
    verify_inputs(base)
    published = read(PUBLISHED_BASE/'results.json')
    for profile, (optimize, guards) in PROFILES.items():
        baseline = base/'profiles'/profile
        if digest(baseline/'probe.image.json') != published['profiles'][profile]['compiler_image_sha256']:
            raise ValueError('Rebuilt baseline differs from its published image: '+profile)
        destination = output/profile
        destination.mkdir(parents=True, exist_ok=True)
        subprocess.run([str(binary), str(baseline), str(base/'exec'), str(destination/'probe'),
                        str(optimize).lower(), str(guards).lower(), str(base/'compiler')], check=True)
        require_projection(read(destination/'program.a816.json'), read(destination/'probe.image.json'))
        summary, rows = inventory(read(destination/'probe.image.json'), read(destination/'probe.inventory.json'))
        save(destination/'inventory-summary.json', summary)
        write_csv(destination/'routines.csv', rows)
    verify_inputs(base)


def write_csv(path, rows):
    with path.open('w', newline='') as file:
        writer = csv.DictWriter(file, fieldnames=list(rows[0]), lineterminator='\n')
        writer.writeheader()
        writer.writerows(rows)


def profile_comparison(before, after, placement):
    for field in ('abi', 'task_headroom', 'irq_headroom'):
        if before[field] != after[field]:
            raise ValueError('Changed ABI/domain obligation: '+field)
    data = lambda image: sorted((r['id'], r['address'], r['size']) for r in image['data'])
    if data(before) != data(after):
        raise ValueError('Changed data identities or reservations')
    old, new = ({r['name']: r for r in image['routines']} for image in (before, after))
    if old.keys() != new.keys():
        raise ValueError('Changed routine census')
    changes = []
    for name, prior in old.items():
        current = new[name]
        for field in ('fixed_frame', 'spill_bytes', 'local_stack_peak'):
            if current[field] > prior[field]:
                raise ValueError('Routine resource growth: '+name+'/'+field)
        changes.append(dict(name=name, before=prior['size'], after=current['size'],
                            delta=current['size']-prior['size']))
    representatives = {}
    for name in REPRESENTATIVES:
        matches = [n for n in old if n.startswith('M_'+name.replace('.', '_')+'_')]
        if len(matches) != 1:
            raise ValueError('Missing or ambiguous representative: '+name)
        n = matches[0]
        representatives[name] = dict(before=old[n]['size'], after=new[n]['size'])
    benefited = sorted({name.split('.')[0] for name, row in representatives.items()
                        if row['after'] < row['before']})
    census = {r['name']: r for r in placement['routines']}
    benefits = {kind: [name for name in old if census.get(name, {}).get(field, 0)
                      and new[name]['size'] < old[name]['size']]
                for kind, field in (('loop', 'loop_homes'), ('call', 'call_segments'))}
    return dict(before_bytes=sum(r['size'] for r in old.values()),
                after_bytes=sum(r['size'] for r in new.values()),
                representative_after=sum(r['after'] for r in representatives.values()),
                representatives=representatives, benefited_subsystems=benefited,
                benefits=benefits, growth=[r for r in changes if r['delta'] > 0],
                frame_or_local_peak_growth=0), changes


def acceptance(profiles, hosted, targets):
    release = profiles['optimized-release']
    checks = {}
    for key, value in (
            ('release_compiler_code_bytes_max', release['after_bytes']),
            ('representative_release_code_bytes_max', release['representative_after']),
            ('native_release_private_accesses_max', release['native']['after']['private']),
            ('native_release_cycles_max', release['native']['after']['cycles'])):
        checks[key] = dict(actual=value, maximum=targets[key], pass_=value <= targets[key])
    checks['subsystems'] = dict(actual=release['benefited_subsystems'], pass_=len(release['benefited_subsystems']) >= 3)
    for kind in ('loop', 'call'):
        checks[kind+'_benefits'] = dict(actual=len(release['benefits'][kind]), pass_=bool(release['benefits'][kind]))
    checks['optimized_native_peak'] = dict(
        actual=max(profiles[p]['native']['after']['peak'] for p in ('optimized-release', 'optimized-guarded')),
        maximum=targets['optimized_native_stack_peak_max'])
    checks['optimized_native_peak']['pass_'] = checks['optimized_native_peak']['actual'] <= checks['optimized_native_peak']['maximum']
    checks['hosted_release'] = dict(pass_=all(hosted.get(c+'-optimized-release', {}).get('status') == 'pass'
                                             for c in ('demo', 'of816')),
                                   requirement='Actual unchecked package under a valid provider contract')
    checks['hosted_regressions'] = dict(
        actual=[key for key, row in hosted.items()
                if row.get('disposition', {}).get('status') in ('new-failure', 'unexplained-failure')])
    checks['hosted_regressions']['pass_'] = not checks['hosted_regressions']['actual']
    return dict(passed=all(r['pass_'] for r in checks.values()), checks=checks,
                failed=[key for key, row in checks.items() if not row['pass_']])


def verified_build(directory):
    marker = read(directory/'qualification-build.json')
    for relative, expected in marker['artifacts'].items():
        if digest(directory/relative) != expected:
            raise ValueError('Changed packaged build artifact: '+relative)
    if marker['compiler_inputs'] != source_hashes():
        raise ValueError('Packaged build belongs to a different compiler/tool generation')
    return marker


def score(base, output):
    verify_inputs(base)
    baseline = read(PUBLISHED_BASE/'results.json')
    result = dict(schema=1, stage=7, compiler_revision=subprocess.check_output(
        ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        baseline_revision=baseline['compiler_revision'], exec_revision=baseline['exec_revision'],
        frozen_inputs_sha256=digest(base/'inputs.json'), targets=baseline['targets']['final'],
        profiles={}, hosted={}, scope='Independent native VM and pinned hosted emulator; no hardware claim')
    rows = []
    for profile in PROFILES:
        directory = output/profile
        marker = verified_build(directory)
        summary, changes = profile_comparison(read(base/'profiles'/profile/'probe.image.json'),
            read(directory/'probe.image.json'), read(directory/'probe.placement.json'))
        reference = read(output/f'{profile}.reference.measurements.json')
        baseline_native(profile, reference)
        summary['native'] = compare_vectors(reference, read(output/f'{profile}.measurements.json'))
        summary['artifact_hashes'] = {name: digest(directory/name) for name in
            ('probe.image.json', 'probe.inventory.json', 'probe.placement.json', 'program.a816.json')}
        summary['hosted_status'] = marker['status']
        if marker['status'] == 'packaged':
            summary['artifact_hashes'].update({name: digest(directory/name) for name in
                ('program.xex', 'exec816-demo.zip', 'of816/Exec-of816.xex')})
            build_record = read(directory/'build.json')
            old_build = read(base/'profiles'/profile/'build.json')
            for field in ('bank_zero_budget', 'task_pools', 'runtime_reservations'):
                if build_record['memory'][field] != old_build['memory'][field]:
                    raise ValueError('Hosted reservations changed: '+field)
            summary['bank_zero_bytes_added'] = 0
            summary['memory'] = {field: build_record['memory'][field]
                                 for field in ('bank_zero_budget', 'task_pools', 'runtime_reservations')}
            packaged = read(directory/'program.a816.json')
            require_projection(packaged, read(directory/'probe.image.json'))
            summary['upper_image_code_plus_initialized_data'] = sum(len(s['bytes']) for s in packaged['segments'])
        result['profiles'][profile] = summary
        rows.extend(dict(profile=profile, **r) for r in changes)
    for profile in ('optimized-guarded', 'raw-guarded'):
        for case in CASES:
            key = case+'-'+profile
            path = output/'hosted'/key/'results.json'
            record = read(path)
            if record['compiler_sha256'] != read(output/profile/'qualification-build.json')['compiler_sha256']:
                raise ValueError('Hosted check used a different compiler: '+key)
            if record['compiler_inputs'] != source_hashes():
                raise ValueError('Hosted check belongs to a different compiler/tool generation: '+key)
            for name, expected in record['artifact_hashes'].items():
                if digest(Path(name)) != expected:
                    raise ValueError('Changed executed hosted artifact: '+name)
            disposition = hosted_disposition(case, 'opt' if profile == 'optimized-guarded' else 'raw',
                record, baseline['hosted'][case+('-opt' if profile == 'optimized-guarded' else '-raw')])
            result['hosted'][key] = dict(status=record['status'], result_sha256=digest(path),
                                        error=record.get('error'), disposition=disposition,
                                        artifact_hashes=record['artifact_hashes'])
    result['acceptance'] = acceptance(result['profiles'], result['hosted'], result['targets'])
    save(output/'score.json', result)
    write_csv(output/'routine-changes.csv', rows)
    return result


def checked_summaries(text):
    rows = [re.search(r'test result: ok\. (\d+) passed; 0 failed; (\d+) ignored', line)
            for line in text.splitlines()]
    rows = [r for r in rows if r]
    if not rows or 'test result: FAILED.' in text:
        raise ValueError('Missing successful final test summaries')
    return dict(targets=len(rows), passed=sum(int(r[1]) for r in rows), ignored=sum(int(r[2]) for r in rows))


def publish(base, output, destination):
    result = score(base, output)
    destination.mkdir(parents=True, exist_ok=True)
    result['validation'] = {}
    for label in ('unit-final', 'integration', 'native-backend'):
        log = output/(label+'.log')
        result['validation'][label] = checked_summaries(log.read_text())
    for label in ('native-backend', *PROFILES, *(p+'.reference' for p in PROFILES)):
        log = output/(label+'.native.log') if label != 'native-backend' else output/'native-backend.log'
        matches = [line.split('Qualification manifest: ', 1)[1] for line in log.read_text().splitlines()
                   if line.startswith('Qualification manifest: ')]
        if len(matches) != 1:
            raise ValueError('Missing exact native qualification: '+label)
        path = Path(matches[0])
        manifest = read(path)
        for relative, expected in manifest['compiler_and_fixture_inputs'].items():
            if digest(ROOT/relative) != expected:
                raise ValueError('Changed native qualified input: '+relative)
        (destination/(label+'.qualification.json.gz')).write_bytes(gzip.compress(path.read_bytes(), mtime=0))
        result['validation'][label+'-manifest'] = digest(path)
    result['host'] = read(output/'host-results.json')
    for key, row in result['hosted'].items():
        path = output/'hosted'/key/'results.json'
        (destination/(key+'.hosted.json.gz')).write_bytes(gzip.compress(path.read_bytes(), mtime=0))
    save(destination/'compiler-inputs.json', source_hashes())
    result['compiler_inputs_sha256'] = digest(destination/'compiler-inputs.json')
    save(destination/'results.json', result)
    (destination/'routine-changes.csv').write_bytes((output/'routine-changes.csv').read_bytes())
    save(destination/'evidence-sha256.json', {p.name: digest(p) for p in destination.iterdir()
                                           if p.name not in ('README.md', 'evidence-sha256.json')})
    print('Final acceptance:', 'pass' if result['acceptance']['passed'] else 'not met')
    print('Open gates:', ', '.join(result['acceptance']['failed']))
    return result['acceptance']['passed']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('build', 'probe', 'score', 'publish'))
    parser.add_argument('--base', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--compiler-root', type=Path, default=ROOT)
    parser.add_argument('--destination', type=Path, default=ROOT/'docs/benchmarks/65816-record-placement-stage7')
    args = parser.parse_args()
    base, output = args.base.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if args.operation in ('build', 'probe') and not args.binary:
        parser.error('--binary required')
    if args.operation == 'build':
        build(base, output, args.compiler_root, args.binary.resolve())
    elif args.operation == 'probe':
        probe(base, output, args.binary.resolve())
    elif args.operation == 'score':
        result = score(base, output)
        print(json.dumps(result['acceptance'], indent=2))
        return 0 if result['acceptance']['passed'] else 1
    else:
        return 0 if publish(base, output, args.destination.resolve()) else 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
