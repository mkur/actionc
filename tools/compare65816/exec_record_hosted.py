#!/usr/bin/env python3
"""Focused hosted stage-0 workloads; each result records the actual pinned machine."""
import argparse
import json
import os
from pathlib import Path
import sys
import time

sys.dont_write_bytecode = True
from exec_record_baseline import verify_inputs, save


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base', type=Path, required=True)
    parser.add_argument('--case', choices=('lists', 'ports', 'dos-streams', 'dos-routing', 'cooked', 'demo', 'of816'), required=True)
    parser.add_argument('--mode', choices=('raw', 'opt'), required=True)
    args = parser.parse_args()
    base = args.base.resolve()
    verify_inputs(base)
    os.environ.update(CARGO_INCREMENTAL='0', CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_DEV_OPT_LEVEL='3')
    sys.path.insert(0, str(base / 'exec/tools'))
    import native_program as native
    toolchain = native.compiler(base / 'compiler')
    # Tests importing compiler see the same already-built and hashed toolchain.
    native.compiler = lambda *a, **kw: toolchain
    out = base / 'hosted' / f'{args.case}-{args.mode}'
    out.mkdir(parents=True, exist_ok=True)
    marker = out / 'results.json'
    marker.unlink(missing_ok=True)
    start = time.monotonic()
    result = dict(status='running', case=args.case, mode=args.mode)
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
            profile = 'optimized-guarded' if args.mode == 'opt' else 'raw-guarded'
            bundle = base / 'profiles' / profile
            boot = json.loads((bundle / 'of816/of816.json').read_text())
            result.update(fixture.check_autoboot(bundle / 'of816', boot, native.read_build(bundle)))
        elif args.case == 'demo':
            import test_demo as fixture
            profile = 'optimized-guarded' if args.mode == 'opt' else 'raw-guarded'
            result.update(fixture.run(base / 'profiles' / profile, boot_smoke=True))
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
        result.update(status='fail', error=str(error))
        raise
    finally:
        result['elapsed_seconds'] = time.monotonic() - start
        save(marker, result)
    verify_inputs(base)
    print(f'Passed {args.case}-{args.mode}', flush=True)


if __name__ == '__main__':
    main()
