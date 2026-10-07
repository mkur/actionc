#!/usr/bin/env python3
"""Focused hosted stage-0 workloads; each result records the actual pinned machine."""
import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import sys
import subprocess
import time

sys.dont_write_bytecode = True
from exec_record_baseline import verify_inputs, save
from exec_record_qualification import digest, pinned_toolchain, source_hashes
from exec_record_frame_maps import install as install_frame_maps


def measured_emulator(original, verify):
    """Add actual machine readback to a frozen helper lacking that check."""
    @contextmanager
    def checked(bridge_dir, rom, output_dir, *, pin):
        with original(bridge_dir, rom, output_dir, pin=pin) as bridge:
            verify(bridge, rom, pin)
            yield bridge
    return checked


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base', type=Path, required=True)
    parser.add_argument('--case', choices=('lists', 'ports', 'dos-streams', 'dos-routing', 'cooked', 'demo', 'of816'), required=True)
    parser.add_argument('--mode', choices=('raw', 'opt'), required=True)
    parser.add_argument('--compiler-root', type=Path)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--profiles', type=Path, help='Measured candidate packages; defaults to frozen baseline')
    parser.add_argument('--output', type=Path, help='Separate hosted evidence directory')
    args = parser.parse_args()
    base = args.base.resolve()
    verify_inputs(base)
    os.environ.update(CARGO_INCREMENTAL='0', CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_DEV_OPT_LEVEL='3')
    sys.path.insert(0, str(base / 'exec/tools'))
    import native_program as native
    if bool(args.compiler_root) != bool(args.binary):
        parser.error('--compiler-root and --binary must be supplied together')
    inputs = source_hashes()
    toolchain = (pinned_toolchain(native, args.compiler_root, args.binary) if args.binary
                 else native.compiler(base / 'compiler'))
    if args.binary:
        install_frame_maps()
    # Tests importing compiler see the same already-built and hashed toolchain.
    native.compiler = lambda *a, **kw: toolchain
    profile = 'optimized-guarded' if args.mode == 'opt' else 'raw-guarded'
    profiles = args.profiles.resolve() if args.profiles else base / 'profiles'
    out = (args.output.resolve() / f'{args.case}-{profile}' if args.output
           else base / 'hosted' / f'{args.case}-{args.mode}')
    out.mkdir(parents=True, exist_ok=True)
    marker = out / 'results.json'
    marker.unlink(missing_ok=True)
    start = time.monotonic()
    result = dict(status='running', case=args.case, mode=args.mode, profile=profile,
                  compiler_sha256=toolchain['binary_sha256'], compiler_revision=toolchain['revision'],
                  compiler_inputs=inputs, frozen_inputs_sha256=digest(base/'inputs.json'),
                  artifact_hashes={}, verified_machines=[])
    original_build, original_machine, original_command = native.build, native.verify_machine, native.command

    def measured_build(*a, **kw):
        program = original_build(*a, **kw)
        for name in ('program.xex', 'program.a816.json', 'build.json', 'layout.json'):
            path = program['output']/name
            result['artifact_hashes'][str(path)] = digest(path)
        return program

    def measured_machine(*a, **kw):
        machine = original_machine(*a, **kw)
        result['verified_machines'].append(machine)
        return machine

    def measured_command(arguments, **kw):
        if str(arguments[0]) == str(toolchain['binary']):
            kw.setdefault('stderr', subprocess.PIPE)
        return original_command(arguments, **kw)

    native.build, native.verify_machine, native.command = measured_build, measured_machine, measured_command
    try:
        if args.case == 'cooked':
            import test_cooked_line as fixture
            result.update(fixture.run(out, args.mode == 'opt', 1))
        elif args.case in ('dos-streams', 'dos-routing'):
            import test_dos_streams as fixture
            if args.case == 'dos-routing':
                result.update(fixture.fixture(toolchain, out, args.mode, 'dos_streams_routing.act'))
            else:
                result.update(fixture.defaults(toolchain, out, args.mode, 1))
        elif args.case == 'of816':
            import test_of816 as fixture
            bundle = profiles / profile
            boot = json.loads((bundle / 'of816/of816.json').read_text())
            original_emulator = fixture.emulator
            fixture.emulator = measured_emulator(original_emulator, measured_machine)
            try:
                result.update(fixture.check_autoboot(bundle / 'of816', boot, native.read_build(bundle)))
            finally:
                fixture.emulator = original_emulator
        elif args.case == 'demo':
            import test_demo as fixture
            result.update(fixture.run(profiles / profile, boot_smoke=True))
        else:
            from os_boundary import emulator
            if args.case == 'lists':
                import test_lists as fixture
                action = lambda bridge: fixture.sequential_case(bridge, toolchain, out, args.mode == 'opt', True)
            else:
                import test_ports_services as fixture
                action = lambda bridge: fixture.queues(bridge, toolchain, out, args.mode == 'opt')
            rom = base / 'exec/build/firmware/altirraos-816.rom'
            bridge_dir = base / 'exec/build/altirra-irq-bridge'
            native.platform_files(bridge_dir, rom)
            with emulator(bridge_dir, rom, out, pin=fixture.PIN) as bridge:
                machine = native.verify_machine(bridge, rom, fixture.PIN)
                result.update(action(bridge), machine=machine)
        result['status'] = 'pass'
    except Exception as error:
        result.update(status='fail', error=str(error), diagnostic=getattr(error, 'stderr', None))
        raise
    finally:
        native.build, native.verify_machine, native.command = original_build, original_machine, original_command
        result['elapsed_seconds'] = time.monotonic() - start
        if args.case in ('demo', 'of816'):
            for name in ('program.xex', 'program.a816.json', 'demo-manifest.json',
                         'of816/Exec-of816.xex', 'of816/of816.json', 'exec816-demo.zip'):
                path = profiles/profile/name
                if path.is_file(): result['artifact_hashes'][str(path)] = digest(path)
        for name in ('observations.bin', 'trace.json', 'expected.bin', 'observed.bin'):
            path = out/name
            if path.is_file(): result['artifact_hashes'][str(path)] = digest(path)
        # Some independent fixtures append diagnostic storage after compilation.
        # Bind the final files actually executed, as well as the compiler inputs.
        result['artifact_hashes'] = {name: digest(Path(name)) for name in result['artifact_hashes']}
        if inputs != source_hashes() or digest(toolchain['binary']) != toolchain['binary_sha256']:
            result.update(status='fail', error='Compiler/tool generation changed during hosted execution')
        save(marker, result)
    verify_inputs(base)
    if result['status'] != 'pass':
        raise ValueError(result['error'])
    print(f'Passed {args.case}-{args.mode}', flush=True)


if __name__ == '__main__':
    main()
