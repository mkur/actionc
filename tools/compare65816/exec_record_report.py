#!/usr/bin/env python3
"""Publish compact stage-0 evidence; bulky images and listings stay under target/."""
import argparse
import csv
import gzip
import json
from pathlib import Path
import shutil

from exec_record_baseline import PROFILES, digest, read, save, verify_inputs

REPRESENTATIVES = ('EXECLISTS.INSERT', 'EXECLISTS.ENQUEUE', 'TASKPOLICY.CREATE',
    'TASKPOLICY.PRODUCERCONTROL', 'PORTCORE.CREATEMSGPORT', 'DOSSTREAMS.OPENNIL',
    'DOSSTREAMS.VALIDBUFFER', 'COOKEDLINE.RECALL', 'COOKEDLINE.RENDER',
    'SDFSFILE.DRIVE', 'SDFSFILE.MEASURE')


def write_csv(path, rows):
    with path.open('w', newline='') as file:
        writer = csv.DictWriter(file, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    base, output = args.base.resolve(), args.output.resolve()
    facts = verify_inputs(base)
    output.mkdir(parents=True, exist_ok=True)
    # Publication requires the measured host run and all native profile results.
    timing = read(base / 'host-timing/results.json')
    for profile in PROFILES:
        if any(not row['correct'] for row in read(base / 'native-vectors' / f'{profile}.measurements.json')['measurements']):
            raise ValueError('Incorrect native results: ' + profile)
    result = dict(schema=1, compiler_revision=facts['compiler']['revision'], exec_revision=facts['exec']['revision'],
        compiler_manifest_sha256=facts['compiler']['manifest_sha256'], exec_manifest_sha256=facts['exec']['manifest_sha256'],
        status='Baseline captured; unchecked hosted release gate remains open', profiles={}, native={}, representatives={},
        host=timing, host_environment=facts['host'], hosted={})
    routine_rows, module_rows, measurements, representative_rows = [], [], [], []
    for profile in PROFILES:
        directory = base / 'profiles' / profile
        summary = read(directory / 'inventory-summary.json')
        summary['compiler_image_sha256'] = digest(directory / 'probe.image.json')
        summary['packaged_image_sha256'] = digest(directory / 'program.a816.json')
        summary['logical_inventory_sha256'] = digest(directory / 'probe.inventory.json')
        summary['source_files'] = len(read(directory / 'probe.inventory.json')['source_paths'])
        summary['compiler_sha256'] = timing['binary_sha256']
        if summary['hosted_status'] == 'packaged':
            summary['program_xex_sha256'] = digest(directory / 'program.xex')
            summary['of816_xex_sha256'] = digest(directory / 'of816/Exec-of816.xex')
            summary['distribution_sha256'] = digest(directory / 'exec816-demo.zip')
            summary['memory'] = read(directory / 'build.json')['memory']
        else:
            summary['hosted_error'] = read(directory / 'hosted-build-failure.json')['error']
        result['profiles'][profile] = summary
        with (directory / 'routines.csv').open() as file:
            routine_rows.extend(dict(profile=profile, **row) for row in csv.DictReader(file))
        module_rows.extend(dict(profile=profile, module=name, bytes=count) for name, count in summary['modules'].items())
        image = read(directory / 'probe.image.json')
        logical = {r['id']: r for r in read(directory / 'probe.inventory.json')['routines']}
        selected = []
        for name in REPRESENTATIVES:
            routine = next(r for r in image['routines'] if r['name'].startswith('M_' + name.replace('.', '_') + '_'))
            blocks = logical[routine['id']]['blocks']
            selected.append(dict(name=name, emitted_name=routine['name'], bytes=routine['size'],
                frame=routine['fixed_frame'], spills=routine['spill_bytes'], local_peak=routine['local_stack_peak'],
                calls=len(routine['calls']), blocks=len(blocks)))
            representative_rows.append(dict(profile=profile, **selected[-1]))
        result['representatives'][profile] = dict(code_bytes=sum(r['bytes'] for r in selected), routines=selected)
        raw = read(base / 'native-vectors' / f'{profile}.measurements.json')['measurements']
        native = dict(vectors=len(raw), cycles=sum(m['cycles'] for m in raw),
            stack_reads=sum(m['stack_reads'] for m in raw), stack_writes=sum(m['stack_writes'] for m in raw),
            dp_reads=sum(m['dp_reads'] for m in raw), dp_writes=sum(m['dp_writes'] for m in raw),
            guard_cycles=sum(m['stack_check_cycles'] for m in raw), max_peak_below_entry_s=max(m['peak_below_entry_s'] for m in raw))
        native['private_byte_accesses'] = sum(native[k] for k in ('stack_reads', 'stack_writes', 'dp_reads', 'dp_writes'))
        native['cases'] = {}
        for case in sorted({m['case'] for m in raw}):
            rows = [m for m in raw if m['case'] == case]
            native['cases'][case] = dict(vectors=len(rows), cycles=sum(m['cycles'] for m in rows),
                private_byte_accesses=sum(m['stack_reads'] + m['stack_writes'] + m['dp_reads'] + m['dp_writes'] for m in rows))
        result['native'][profile] = native
        keys = ('case', 'vector', 'cycles', 'instructions', 'stack_reads', 'stack_writes', 'dp_reads', 'dp_writes',
                'stack_check_cycles', 'stack_check_instructions', 'peak_below_entry_s', 'correct')
        measurements.extend(dict(profile=profile, **{k: m[k] for k in keys}) for m in raw)
    for path in sorted((base / 'hosted').glob('*/results.json')):
        item = read(path)
        runtime = item.get('runtime', {})
        keep = ('guards', 'status', 'native_nmi_count', 'native_irq_count', 'switches', 'created',
                'stack_observations', 'kernel_stack_observation')
        row = dict(status=item['status'], result_sha256=digest(path), elapsed_seconds=item['elapsed_seconds'],
                   runtime={k: runtime[k] for k in keep if k in runtime})
        for key in ('error', 'operations', 'checks', 'observations', 'scope', 'frames', 'os_restored', 'guards', 'bank_zero_delta'):
            if key in item: row[key] = item[key]
        result['hosted'][path.parent.name] = row
    result['targets'] = dict(foundation=dict(compiler_image_changes=0, frame_map_changes=0,
        bank_zero_bytes_added=0, host_wall_ratio_max=1.05, host_peak_rss_ratio_max=1.10),
        first_benefit=dict(representative_code_ratio_max=0.98, whole_exec_code_ratio_max=1.0,
            requirement='General independent probe benefit plus at least two Exec subsystems'),
        final=dict(release_compiler_code_bytes_max=422700, representative_release_code_bytes_max=9235,
            native_release_private_accesses_max=20565, native_release_cycles_max=78117,
            individual_vector_cycle_ratio_max=1.05, optimized_native_stack_peak_max=42,
            frame_or_local_peak_growth=0, bank_zero_bytes_added=0,
            requirement='Benefits in at least three Exec subsystems, including looping and calling routines; no new hosted failures'))
    result['measurement_tools'] = {str(p.relative_to(Path(__file__).resolve().parents[2])): digest(p)
        for p in [*Path(__file__).parent.glob('exec_record*.py'), Path(__file__).with_name('freeze_exec_baseline.py'),
                  *Path(__file__).with_name('record_probe').rglob('*'),
                  Path(__file__).parent.parent / 'native65816-runtime-tests/tests/code_quality.rs']
        if p.is_file() and '__pycache__' not in p.parts}
    result['probe_binary_sha256'] = digest(base / 'rust-target/debug/actionc-exec-record-probe')
    runtime = Path(__file__).parent.parent / 'native65816-runtime-tests'
    vm = runtime / 'target' / ('qualified-vm-' + digest(runtime / 'vm-status-timing.patch')[:16])
    result['vm'] = read(vm / 'qualification-source.json')
    result['host']['conditions'] = 'Warm cache on shared macOS host; other development work was running. Recompare candidates interleaved on the same host.'
    save(output / 'results.json', result)
    write_csv(output / 'routines.csv', routine_rows)
    write_csv(output / 'modules.csv', module_rows)
    write_csv(output / 'native-metrics.csv', measurements)
    write_csv(output / 'representatives.csv', representative_rows)
    shutil.copyfile(base / 'native-vectors/sources.json', output / 'native-source-hashes.json')
    (output / 'inputs.json.gz').write_bytes(gzip.compress((base / 'inputs.json').read_bytes(), mtime=0))
    save(output / 'evidence-sha256.json', {p.name: digest(p) for p in output.iterdir()
        if p.is_file() and p.name not in ('README.md', 'evidence-sha256.json')})
    print('Published', output)


if __name__ == '__main__': main()
