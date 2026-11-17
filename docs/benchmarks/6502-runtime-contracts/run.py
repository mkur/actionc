#!/usr/bin/env python3
"""Qualify runtime contracts using the existing TN and Huffman fact probes.

Do not build or edit known_callees.rs concurrently. Large outputs stay in build/.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'build/runtime-contracts-qualification'
HOOK = b'''\n#[cfg(test)]
mod runtime_contract_audit { include!(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/benchmarks/6502-huffman-facts/probe.rs")); }
'''


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    (ROOT / 'build/call-facts-audit-20261116').mkdir(parents=True, exist_ok=True)
    driver = ROOT / 'build/huffman-facts/driver'
    driver.mkdir(parents=True, exist_ok=True)
    for source, name in [
        ('tools/vm-runtime-tests/fixtures/huff_dec_driver.act', 'huff_dec.act'),
        ('fixtures/runtime/tacle/huff_dec/types.inc', 'types.inc'),
        ('fixtures/runtime/tacle/huff_dec/kernel.inc', 'kernel.inc'),
    ]:
        (driver / name).write_text((ROOT / source).read_text().replace('\r\n', '\n'))
    module = ROOT / 'src/mir6502/analysis/known_callees.rs'
    original = module.read_bytes()
    assert b'mod runtime_contract_audit' not in original
    env = os.environ.copy()
    env.setdefault('CARGO_TARGET_DIR', str(ROOT / 'build/checked-arena/shared-target'))
    env.update(CARGO_BUILD_JOBS='2', CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_TEST_DEBUG='0',
               CARGO_PROFILE_DEV_OPT_LEVEL='1', CARGO_PROFILE_TEST_OPT_LEVEL='1',
               ACTIONC_CALL_FACT_PROFILE='1', ACTIONC_CALL_FACT_DETAILS='1')
    command = ['cargo', 'test', '--lib',
               'mir6502::analysis::known_callees::runtime_contract_audit::audit_',
               '--', '--ignored', '--nocapture', '--test-threads=1']
    receipt = {'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
               'module_sha256': hashlib.sha256(original).hexdigest(), 'command': command}
    module.write_bytes(original + HOOK)
    try:
        receipt['return_code'] = subprocess.run(command, cwd=ROOT, env=env).returncode
    finally:
        current = module.read_bytes()
        assert current.count(HOOK) == 1
        module.write_bytes(current.replace(HOOK, b'', 1))
        receipt['hook_removed'] = module.read_bytes() == original
        (OUT / 'run.json').write_text(json.dumps(receipt, indent=2) + '\n')
    raise SystemExit(receipt['return_code'])


if __name__ == '__main__':
    main()
