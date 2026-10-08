#!/usr/bin/env python3
"""Authenticate changed call-flow candidates separately from the frozen audit."""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

from exec_call_audit import FILES, ROOT, audit_profile, compiler_inputs, csv_text, require
from exec_record_baseline import PROFILES, digest, read, save, verify_inputs

CANDIDATE_FILES = (*FILES, 'probe.flow.json')


def sources():
    result = compiler_inputs()
    paths = [*ROOT.joinpath('tools/native65816-runtime-tests/tests').rglob('*'),
             Path(__file__), Path(__file__).with_name('test_exec_call_candidate.py'),
             Path(__file__).with_name('exec_call_audit.py'),
             Path(__file__).with_name('exec_record_baseline.py'),
             ROOT/'tools/disassemble65816.py',
             ROOT/'tools/native65816-runtime-tests/qualify.py',
             ROOT/'tools/native65816-runtime-tests/vm-status-timing.patch']
    result.update({str(p.relative_to(ROOT)): digest(p) for p in paths if p.is_file()})
    return result


def authenticate(output):
    facts = read(output/'provenance.json')
    require(facts.get('schema') == 1 and facts.get('kind') == 'call-flow-candidate', 'Unknown candidate provenance')
    require(set(facts['profiles']) == set(PROFILES), 'Incomplete candidate profiles')
    for profile, row in facts['profiles'].items():
        require(set(row['artifacts']) == set(CANDIDATE_FILES), 'Incomplete candidate artifacts')
        for name, expected in row['artifacts'].items():
            require(digest(output/profile/name) == expected, 'Candidate artifact drift: '+profile+'/'+name)
    return facts


def resource_deltas(before, after):
    old = {r['routine']: r for r in before}
    new = {r['routine']: r for r in after}
    require(len(old) == len(before) and len(new) == len(after) and old.keys() == new.keys(), 'Routine identity drift')
    rows = []
    for name, row in new.items():
        delta = {key: row[key]-old[name][key] for key in ('code_bytes', 'frame', 'spill', 'local_peak')}
        require(all(delta[k] <= 0 for k in ('frame', 'spill', 'local_peak')), 'Routine resource growth: '+name)
        if any(delta.values()):
            rows.append(dict(routine=name, **delta))
    return rows


def collect(base, output, stage):
    verify_inputs(base)
    require(not (output/'provenance.json').exists(), 'Use a fresh candidate generation')
    before = sources()
    cache = ROOT/'target/exec-call-audit/rust-target'
    build = ['cargo', 'build', '--locked', '--manifest-path', str(ROOT/'tools/compare65816/record_probe/Cargo.toml'),
             '--features', 'flow-analysis', '--target-dir', str(cache), '-j2']
    settings = dict(CARGO_INCREMENTAL='0', CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_DEV_OPT_LEVEL='3')
    subprocess.run(build, check=True, cwd=ROOT, env=dict(os.environ, **settings))
    output.mkdir(parents=True, exist_ok=True)
    binary = output/'record-probe'
    shutil.copyfile(cache/'debug/actionc-exec-record-probe', binary)
    binary.chmod(0o755)
    facts = dict(schema=1, kind='call-flow-candidate', stage=stage,
                 compiler_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                 rust=subprocess.check_output(['rustc', '--version'], cwd=ROOT, text=True).strip(),
                 compiler_and_fixture_inputs=before, probe_binary_sha256=digest(binary),
                 build_command=build, build_environment=settings,
                 frozen_inputs_sha256=digest(base/'inputs.json'), profiles={})
    original = read(ROOT/'docs/benchmarks/65816-exec-call-audit/provenance.json')
    for profile, (optimize, guards) in PROFILES.items():
        directory = output/profile
        directory.mkdir()
        generated = base/'profiles'/profile
        generated_hashes = {str(p.relative_to(generated)): digest(p) for p in generated.rglob('*')
                            if p.is_file() and (p.suffix in ('.act', '.inc') or p.name == 'layout.json')}
        command = [str(binary), str(generated), str(base/'exec'), str(directory/'probe'),
                   str(optimize).lower(), str(guards).lower(), str(ROOT)]
        subprocess.run(command, check=True, cwd=ROOT)
        require(generated_hashes == {name: digest(generated/name) for name in generated_hashes}, 'Generated source drift')
        artifacts = {name: digest(directory/name) for name in CANDIDATE_FILES}
        if stage == 0:
            require(all(artifacts[name] == original['profiles'][profile]['artifacts'][name] for name in FILES),
                    'Stage-0 frozen artifact mismatch: '+profile)
        inventory = read(directory/'probe.inventory.json')
        facts['profiles'][profile] = dict(command=command, artifacts=artifacts, generated_inputs=generated_hashes,
            actual_source_files={name: digest(Path(name)) for name in inventory['source_paths']})
    require(before == sources() and digest(binary) == facts['probe_binary_sha256'], 'Compiler/fixture/probe drift')
    verify_inputs(base)
    save(output/'provenance.json', facts)


def report(output, reference, destination, check):
    facts = authenticate(output)
    prior = authenticate(reference) if reference else None
    if prior:
        require(prior['stage'] == 0 and prior['frozen_inputs_sha256'] == facts['frozen_inputs_sha256'], 'Wrong baseline lineage')
    profiles, calls, captures, routines, changes = {}, [], [], [], []
    for profile in PROFILES:
        directory = output/profile
        summary, call_rows, routine_rows, _, capture_rows = audit_profile(
            read(directory/FILES[0]), read(directory/FILES[1]), read(directory/FILES[3]), read(directory/FILES[2]))
        profiles[profile] = summary
        for table, rows in ((calls, call_rows), (captures, capture_rows), (routines, routine_rows)):
            table.extend(dict(profile=profile, **row) for row in rows)
        if reference:
            previous = reference/profile
            _, _, before, _, _ = audit_profile(read(previous/FILES[0]), read(previous/FILES[1]),
                                              read(previous/FILES[3]), read(previous/FILES[2]))
            changes.extend(dict(profile=profile, **row) for row in resource_deltas(before, routine_rows))
    result = dict(schema=1, kind='call-flow-candidate', stage=facts['stage'], profiles=profiles,
                  scope='Static complete compiler code and physical call/home accesses; not VM cycles or whole hosted package.',
                  baseline_provenance_sha256=digest(reference/'provenance.json') if reference else None)
    artifacts = {'results.json': (json.dumps(result, indent=2)+'\n').encode(),
                 'provenance.json': (json.dumps(facts, indent=2)+'\n').encode()}
    for name, rows in (('calls', calls), ('captures', captures), ('routines', routines), ('routine-changes', changes)):
        artifacts[name+'.csv.gz'] = gzip.compress(csv_text(rows).encode(), mtime=0)
    artifacts['evidence-sha256.json'] = (json.dumps({name: hashlib.sha256(data).hexdigest()
        for name, data in artifacts.items()}, indent=2)+'\n').encode()
    for name, data in artifacts.items():
        if check:
            require((destination/name).read_bytes() == data, 'Published candidate drift: '+name)
        else:
            destination.mkdir(parents=True, exist_ok=True)
            (destination/name).write_bytes(data)
    print('Candidate verified' if check else 'Candidate written', destination)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('collect', 'report'))
    parser.add_argument('--base', type=Path, default=ROOT/'target/record-placement-stage0')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--stage', type=int, default=0)
    parser.add_argument('--reference', type=Path)
    parser.add_argument('--destination', type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    require(args.stage >= 0, 'Negative stage')
    if args.operation == 'collect':
        require(not args.check, '--check applies to report')
        collect(args.base.resolve(), args.output.resolve(), args.stage)
    else:
        require(args.destination is not None, '--destination is required for report')
        report(args.output.resolve(), args.reference.resolve() if args.reference else None,
               args.destination.resolve(), args.check)


if __name__ == '__main__':
    main()
