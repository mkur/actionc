#!/usr/bin/env python3
"""Build and measure frozen Exec816 inputs for record/value-placement work.

No compiler strategy changes. See the stage-0 scorecard for the frozen inputs,
profile definitions, hosted scope, and numerical gates.
"""
import argparse
from collections import Counter
import csv
import hashlib
import json
import os
from pathlib import Path
import re
import statistics
import shutil
import subprocess
import sys

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from disassemble65816 import disassemble, STACK, DP, BRANCH
from measure_host import run

PROFILES = {'optimized-release': (True, False),
            'optimized-guarded': (True, True), 'raw-guarded': (False, True)}


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def read(path):
    return json.loads(Path(path).read_text())


def save(path, value):
    Path(path).write_text(json.dumps(value, indent=2) + '\n')


def verify_inputs(base):
    """Check the frozen copies, including the deliberate local compiler pin."""
    facts = read(base / 'inputs.json')
    for name in ('compiler', 'exec'):
        for relative, expected in facts[name]['files'].items():
            path = base / name / relative
            if name == 'exec' and relative == 'toolchain/actionc.json':
                if read(path) != facts['compiler_pin_override']['baseline']:
                    raise ValueError('Changed baseline compiler pin')
            elif digest(path) != expected:
                raise ValueError(f'Changed frozen input: {name}/{relative}')
    for relative, expected in facts['external_inputs'].items():
        if digest(base / 'exec/build' / relative) != expected:
            raise ValueError(f'Changed external input: {relative}')
    for relative, expected in facts['of816']['files'].items():
        if digest(base / 'exec/build/of816-upstream' / relative) != expected:
            raise ValueError(f'Changed OF816 input: {relative}')
    return facts


def build(base, profile):
    verify_inputs(base)
    sys.path.insert(0, str(base / 'exec/tools'))
    import build_demo
    # Use one compact, optimized host compiler. This is independent of target
    # source optimization and the native/platform stack guard selection.
    os.environ.update(CARGO_INCREMENTAL='0', CARGO_PROFILE_DEV_DEBUG='0',
                      CARGO_PROFILE_DEV_OPT_LEVEL='3')
    optimize, guards = PROFILES[profile]
    native_build, command_build = build_demo.build, build_demo.compile_command

    def native(*args, **kwargs):
        kwargs.update(optimize=optimize, stack_checks=guards)
        return native_build(*args, **kwargs)

    def command(*args, **kwargs):
        kwargs['optimize'] = optimize
        return command_build(*args, **kwargs)

    build_demo.build, build_demo.compile_command = native, command
    output = base / 'profiles' / profile
    output.mkdir(parents=True, exist_ok=True)
    marker = output / 'baseline-profile.json'
    marker.unlink(missing_ok=True)
    (output / 'hosted-build-failure.json').unlink(missing_ok=True)
    (output / 'program.a816.json').unlink(missing_ok=True)
    (output / 'program.xex').unlink(missing_ok=True)
    error = None
    try:
        result = build_demo.bundle(output, base / 'compiler')
    except ValueError as failure:
        if profile != 'optimized-release' or str(failure) != 'Hosted o65 requires a checked kernel':
            raise
        error = str(failure)
        result = {'kernel': {'binary_sha256': digest(base / 'compiler/target/debug/actionc-65816')}}
        save(output / 'hosted-build-failure.json', dict(status='unsupported', error=error, distribution_built=False))
    finally:
        build_demo.build, build_demo.compile_command = native_build, command_build
    verify_inputs(base)
    save(marker, dict(name=profile, status='unsupported' if error else 'built', error=error, optimize=optimize, stack_checks=guards,
        compiler_host_profile='dev, opt-level=3, debug=0, incremental=false',
        command_stack_checks='o65-experimental provider contract; independent of kernel guards',
        compiler_sha256=result['kernel']['binary_sha256'],
        artifacts={str(p.relative_to(output)): digest(p) for p in output.rglob('*')
                   if p.is_file() and p != marker}))
    print(f'{profile}: {error or output / "exec816-demo.zip"}')


def probe(base, profile):
    verify_inputs(base)
    source = HERE / 'record_probe'
    copied = base / 'probe-tool'
    (copied / 'src').mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source / 'src/main.rs', copied / 'src/main.rs')
    manifest = (source / 'Cargo.toml').read_text().replace('path = "../../.."',
        'path = ' + json.dumps(str(base / 'compiler')))
    (copied / 'Cargo.toml').write_text(manifest)
    shutil.copyfile(source / 'Cargo.lock', copied / 'Cargo.lock')
    env = dict(os.environ, CARGO_INCREMENTAL='0', CARGO_PROFILE_DEV_DEBUG='0',
        CARGO_PROFILE_DEV_OPT_LEVEL='3', CARGO_TARGET_DIR=str(base / 'rust-target'))
    subprocess.run(['cargo', 'build', '--locked', '--manifest-path', str(copied / 'Cargo.toml')], check=True, env=env)
    output = base / 'profiles' / profile
    optimize, guards = PROFILES[profile]
    binary = base / 'rust-target/debug/actionc-exec-record-probe'
    subprocess.run([str(binary), str(output), str(base / 'exec'), str(output / 'probe'),
        str(optimize).lower(), str(guards).lower(), str(base / 'compiler')], check=True)
    account(base, profile)
    verify_inputs(base)


def instructions(image):
    if (image['format'], image['abi'], image['version']) not in [
        ('actionc-65816-image', 'action65816.native.v2', 3),
        ('actionc-65816-image', 'action65816.native.v2', 4)]:
        raise ValueError('Unknown image contract')
    # v4 adds arithmetic-fault metadata; the encoded instruction contract is
    # unchanged. Adapt only the decoder envelope, never the saved artifact.
    for line in disassemble(dict(image, version=3)).splitlines():
        match = re.fullmatch(r'([0-9A-F]{6})  ((?:[0-9A-F]{2} )*[0-9A-F]{2})\s+(.+)', line)
        if not match:
            raise ValueError(f'Invalid listing: {line}')
        yield int(match[1], 16), bytes.fromhex(match[2]), match[3]


def guard_ranges(image):
    """Recognize the current complete native-v2 guard, checked at instruction boundaries."""
    fault = image['stack_overflow'].to_bytes(3, 'little')
    pattern = re.compile(rb'\x3b\xaa\xc5\xc6\x90\x04\xf0\x02\x80\x0a\x38\xe9(..)\x90\x04\xc5\xc4\xb0\x07\xa9\1\x5c' + re.escape(fault), re.S)
    decoded = {pc: code for pc, code, _ in instructions(image)}
    ranges = []
    for segment in image['segments']:
        if not segment['executable']: continue
        for match in pattern.finditer(bytes(segment['bytes'])):
            start = segment['address'] + match.start()
            end = segment['address'] + match.end()
            if start in decoded and end - 4 in decoded:
                ranges.append([start, end])
    jumps = {pc for pc, code in decoded.items() if code == b'\x5c' + fault}
    if jumps != {end - 4 for _, end in ranges}:
        raise ValueError('Unrecognized stack-overflow guard')
    if image.get('stack_checks', True) is False and ranges:
        raise ValueError('Unchecked artifact contains guards')
    return ranges


def inventory(image, logical):
    routines = {r['id']: r for r in image['routines']}
    code = sum(r['size'] for r in routines.values())
    executable = [s for s in image['segments'] if s['executable']]
    if sum(len(s['bytes']) for s in executable) != code:
        raise ValueError('Executable segments/routine inventory disagree')
    categories = Counter()
    by_pc = {}
    for pc, data, text in instructions(image):
        if pc in by_pc:
            raise ValueError('Overlapping executable segments')
        by_pc[pc] = len(data)
        op = data[0]
        category = ('stack_relative' if op in STACK else 'direct_page' if op in DP
            else 'mode_changes' if op in (0xc2, 0xe2) else 'JSL' if op == 0x22
            else 'JML' if op == 0x5c else 'branches' if op in BRANCH or op == 0x82
            else 'indirect_memory' if op in (0xa7, 0x87, 0xb7, 0x97)
            else 'absolute_long_memory' if op in (0xaf, 0x8f, 0xbf, 0x9f) else 'other')
        categories[category] += len(data)
    if sum(categories.values()) != code:
        raise ValueError('Instruction accounting does not cover all routine bytes')
    families = Counter()
    rows = []
    modules = Counter()
    for routine in logical['routines']:
        physical = routines.pop(routine['id'])
        covered = set()
        for span in routine['spans']:
            start, end = span['start'], span['end']
            if not 0 <= start <= end <= physical['size']:
                raise ValueError('MIR span outside routine')
            extent = set(range(start, end))
            if covered & extent:
                raise ValueError('Overlapping MIR spans')
            covered |= extent
            families[span['kind'].split('/')[0]] += end - start
        families['Prologue/helpers/unspanned'] += physical['size'] - len(covered)
        # Display-name grouping is reporting metadata, never executable facts.
        name = physical['name']
        module = name.split('_')[1] if name.startswith('M_') else '<runtime>'
        modules[module] += physical['size']
        rows.append(dict(name=name, module=module, bytes=physical['size'],
            fixed_frame=physical['fixed_frame'], spill_bytes=physical['spill_bytes'],
            local_stack_peak=physical['local_stack_peak'], calls=len(physical['calls']),
            blocks=len(routine['blocks'])))
    if routines or sum(families.values()) != code:
        raise ValueError('Logical inventory does not cover all image routines')
    return dict(code_bytes=code, stack_check_bytes=sum(end-start for start,end in guard_ranges(image)), initialized_data_bytes=sum(len(s['bytes']) for s in image['segments'] if not s['executable']),
        zero_fill_bytes=sum(s['size'] for s in image['zero_fill']), routines=len(rows),
        categories=dict(categories), families=dict(families), modules=dict(modules),
        largest_frame_bytes=max(r['fixed_frame'] for r in rows),
        largest_local_stack_peak=max(r['local_stack_peak'] for r in rows)), rows


def require_projection(packaged, compiler):
    for key in compiler:
        if key not in ('segments', 'data') and packaged[key] != compiler[key]:
            raise ValueError('Compiler/packaged image disagreement: ' + key)
    for key in ('segments', 'data'):
        if any(item not in packaged[key] for item in compiler[key]):
            raise ValueError('Missing or changed compiler-owned ' + key)


def account(base, profile):
    output = base / 'profiles' / profile
    saved = read(output / 'program.a816.json')
    probed = read(output / 'probe.image.json')
    require_projection(saved, probed)
    summary, rows = inventory(probed, read(output / 'probe.inventory.json'))
    summary['hosted_status'] = 'unsupported' if (output / 'hosted-build-failure.json').exists() else 'packaged'
    if summary['hosted_status'] == 'packaged':
        summary['upper_image_code_bytes'] = sum(len(s['bytes']) for s in saved['segments'] if s['executable'])
        summary['upper_image_initialized_data_bytes'] = sum(len(s['bytes']) for s in saved['segments'] if not s['executable'])
        summary['upper_image_code_plus_initialized_data_bytes'] = sum(len(s['bytes']) for s in saved['segments'])
        summary['xex_file_bytes'] = (output / 'program.xex').stat().st_size
        summary['of816_xex_file_bytes'] = (output / 'of816/Exec-of816.xex').stat().st_size
    save(output / 'inventory-summary.json', summary)
    with (output / 'routines.csv').open('w', newline='') as file:
        writer = csv.DictWriter(file, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(sorted(rows, key=lambda row: -row['bytes']))
    (output / 'compiler.lst').write_text(disassemble(dict(probed, version=3)))
    print(profile, summary['code_bytes'], summary['initialized_data_bytes'], summary['routines'])


def host(base, rounds):
    if rounds < 3:
        raise ValueError('At least three timing rounds required')
    verify_inputs(base)
    binary = base / 'compiler/target/debug/actionc-65816'
    sys.path.insert(0, str(base / 'exec/tools'))
    from library_paths import module_args
    samples, commands = [], {}
    timed = base / 'host-timing'
    timed.mkdir(exist_ok=True)
    for profile in PROFILES:
        output = base / 'profiles' / profile
        args = [str(binary), '--module-path', str(output / 'task-kernel'),
                '--module-path', str(output), '--module-path', str(output),
                '--module-path', str(base / 'compiler/runtime/65816'), *map(str, module_args()),
                '--layout', str(output / 'layout.json'), '-o', str(timed / 'image.json'),
                *([] if PROFILES[profile][0] else ['--no-opt']), str(output / 'kernel-program.act')]
        commands[profile] = args
    for iteration in range(-1, rounds):
        order = list(PROFILES) if iteration % 2 else list(reversed(PROFILES))
        for profile in order:
            measurement = run(commands[profile], timed / 'compiler.log')
            if read(timed / 'image.json') != read(base / 'profiles' / profile / 'probe.image.json'):
                raise ValueError(f'Timed compile changed image: {profile}')
            if iteration >= 0:
                samples.append(dict(round=iteration, profile=profile, **measurement))
    verify_inputs(base)
    keys = ('wall_seconds', 'user_seconds', 'system_seconds', 'peak_rss_bytes')
    save(timed / 'results.json', dict(scope='Warm compiler CLI only; excludes interfaces, assembly, media and packaging',
        binary_sha256=digest(binary), rounds=rounds, commands=commands, samples=samples,
        medians={p: {k: statistics.median(s[k] for s in samples if s['profile'] == p) for k in keys} for p in PROFILES}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('verify', 'build', 'probe', 'inventory', 'host'))
    parser.add_argument('--base', type=Path, required=True)
    parser.add_argument('--profile', choices=PROFILES)
    parser.add_argument('--rounds', type=int, default=5)
    args = parser.parse_args()
    base = args.base.resolve()
    if args.operation in ('build', 'probe', 'inventory') and not args.profile:
        parser.error('--profile required')
    if args.operation == 'verify': verify_inputs(base)
    elif args.operation == 'build': build(base, args.profile)
    elif args.operation == 'probe': probe(base, args.profile)
    elif args.operation == 'inventory': account(base, args.profile)
    else: host(base, args.rounds)


if __name__ == '__main__':
    main()
