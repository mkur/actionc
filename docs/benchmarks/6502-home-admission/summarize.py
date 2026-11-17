#!/usr/bin/env python3
"""Record the source-home admission comparison after the existing runtime-contract probe."""
import importlib.util
import json
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
PREVIOUS = HERE.parent / '6502-wide-routine-abi'
HELPERS = HERE.parent / '6502-runtime-contracts'
spec = importlib.util.spec_from_file_location('previous', HELPERS / 'summarize.py')
previous = importlib.util.module_from_spec(spec)
spec.loader.exec_module(previous)
read, sha, summarize, stats = previous.read, previous.sha, previous.summarize, previous.stats



def record_checks():
    logs = {
        'mir6502_units': 'final-units',
        'call_abi_integration': 'final-call-abi',
        'residence_library_abi_and_admission_vm': 'final-controls-vm',
        'existing_shift_vm': 'final-shifts-vm',
        'huffman_vm': 'final-huffman-vm',
        'combined_fact_audit': 'final-audit',
        'focused_parameter_homes': 'slice4',
        'focused_shift_expansion': 'slice4-expansion',
        'helper_contract_units': 'slice2-contracts',
        'wide_fault_vm': 'slice2-fault-vm',
    }
    result = {}
    for label, suffix in logs.items():
        path = ROOT / f'build/home-admission-{suffix}.log'
        text = path.read_text().replace('\r\n', '\n')
        rows = re.findall(r'test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored; \d+ measured; (\d+) filtered out;', text)
        assert rows, (label, path)
        failures = re.findall(r'^    (mir6502::\S+)$', text, re.M)
        result[label] = {'status': 'FAILED' if any(r[0] != 'ok' for r in rows) else 'ok',
            **{name: sum(int(r[i]) for r in rows) for i, name in enumerate(
                ['passed', 'failed', 'ignored', 'filtered_out'], 1)},
            'failures': failures, 'log': str(path.relative_to(ROOT)), 'log_sha256': sha(path)}
    baseline_failures = read(PREVIOUS / 'checks.json')['mir6502_units']['failures']
    assert result['mir6502_units']['failures'] == baseline_failures
    assert all(r['failed'] == 0 for n, r in result.items() if n != 'mir6502_units')
    result['commands'] = read(ROOT / 'build/home-admission-final-runs.json')
    result['unit_command'] = ['build/checked-arena/shared-target/debug/deps/actionc-60432645d6edf052', 'mir6502::', '--test-threads=2']
    result['baseline_failure_receipt'] = '../6502-wide-routine-abi/checks.json'
    result['scope'] = 'MIR6502 units and affected ABI/VM controls; no other backend or repository-wide suite.'
    result['huffman_coverage'] = '185 vectors per provider; 370 executions; actual cartridge LF and standalone CRLF compilation/VM paths.'
    result['tn_execution'] = 'Reused baseline execution: complete selected image and all routine code sizes are byte-identical.'
    result['temporary_audit_hook_removed'] = read(ROOT / 'build/runtime-contracts-qualification/run.json')['hook_removed']
    (HERE / 'checks.json').write_text(json.dumps(result, indent=2) + '\n')


def main():
    run = read(ROOT / 'build/runtime-contracts-qualification/run.json')
    assert run['return_code'] == 0 and run['hook_removed']
    baseline = read(PREVIOUS / 'facts.json')
    baseline_receipt = read(PREVIOUS / 'qualification.json')
    current = {'TN': summarize('TN', ROOT / 'build/call-facts-audit-20261116/tn.json',
                               ROOT / 'build/call-facts-audit-20261116/tn.xex')}
    for label in ['portable', 'driver', 'portable-no-inline']:
        current[label] = summarize(label, ROOT / f'build/huffman-facts/{label}.json',
                                  ROOT / f'build/huffman-facts/{label}.xex')
        read_bits = next(r for r in current[label]['routines'] if r['name'] == 'ReadBits')
        assert read_bits['direct_parameter_abi'] == 'Some([Scalar(Word)])'
        assert read_bits['direct_result_abi'] == 'Some(Scalar(Long))'
        assert read_bits['entry']['current']
        assert read_bits['entry']['defined_results'] == [0xa0, 0xa1, 0xa2, 0xa3]
    receipt = {'production_revision': run['revision'], 'baseline_revision': '6167df8b',
               'baseline_facts_sha256': sha(PREVIOUS / 'facts.json'), 'run': run, 'programs': {}}
    for label, after in current.items():
        before = baseline[label]
        old = {r['name']: r for r in before['routines']}
        now = {r['name']: r for r in after['routines']}
        receipt['programs'][label] = {
            'before': stats(before), 'after': stats(after),
            'byte_delta': after['packaged_bytes'] - before['packaged_bytes'],
            'image_unchanged': after['image_sha256'] == before['image_sha256'],
            'named_homes_unchanged': stats(after)['named_zp_homes'] == stats(before)['named_zp_homes'],
            'temporary_homes_unchanged': stats(after)['temporary_zp'] == stats(before)['temporary_zp'],
            'new_entry_certificates': [n for n, r in now.items()
                                       if r['entry_available'] and not old.get(n, {}).get('entry_available', False)],
            'lost_entry_certificates': [n for n, r in old.items()
                                        if r['entry_available'] and not now.get(n, {}).get('entry_available', False)],
            'changed_routine_code_sizes': [{'name': n, 'before': old[n]['code_bytes'], 'after': r['code_bytes']}
                                         for n, r in now.items() if n in old and old[n]['code_bytes'] != r['code_bytes']],
            'helper_sites': sum(len(r['helper_sites']) for r in after['routines']),
            'available_helper_sites': sum(h['available'] for r in after['routines'] for h in r['helper_sites']),
        }
    receipt['TN_profile'] = read(ROOT / 'build/call-facts-audit-20261116/tn-profile.json')
    receipt['TN_configuration'] = baseline_receipt['TN_configuration']
    receipt['TN_trace'] = read(ROOT / 'build/call-facts-audit-20261116/tn-contract-events.json')
    receipt['huffman_runs'] = {label: read(ROOT / f'build/huffman-facts/{label}-run.json') for label in current if label != 'TN'}
    receipt['huffman_events'] = {label: read(ROOT / f'build/huffman-facts/{label}-events.json') for label in current if label != 'TN'}
    receipt['source_sha256'] = {p: sha(ROOT / p) for p in baseline_receipt['source_sha256']}
    assert receipt['source_sha256'] == baseline_receipt['source_sha256'], 'baseline sources changed'
    receipt['probe_sha256'] = {p: sha(ROOT / p) for p in [
        'docs/benchmarks/6502-call-facts-audit/probe.rs',
        'docs/benchmarks/6502-huffman-facts/probe.rs',
        'docs/benchmarks/6502-runtime-contracts/run.py',
        'docs/benchmarks/6502-runtime-contracts/summarize.py',
        'docs/benchmarks/6502-home-admission/summarize.py',
    ]}
    # Admission changes must preserve established executable contracts.
    for label, program in receipt['programs'].items():
        assert not program['lost_entry_certificates'], (label, program['lost_entry_certificates'])
        assert program['helper_sites'] == program['available_helper_sites']
    for label in ['portable', 'driver', 'portable-no-inline']:
        for name in ['ReadBits', 'ReadHeader', 'Decode']:
            r = next(r for r in current[label]['routines'] if r['name'] == name)
            assert r['entry_available'] and r['entry']['current'], (label, name)
        r = next(r for r in current[label]['routines'] if r['name'] == 'ReadBits')
        value = next(s for s in r['named_homes'] if s['name'] == 'value')
        assert value['address'] >= 256 and value['size'] == 4
    (HERE / 'facts.json').write_text(json.dumps(current, indent=2) + '\n')
    (HERE / 'qualification.json').write_text(json.dumps(receipt, indent=2) + '\n')
    for label, p in receipt['programs'].items():
        print(label, p['before']['packaged_bytes'], '->', p['after']['packaged_bytes'], 'bytes;',
              p['before']['entry_certificate_count'], '->', p['after']['entry_certificate_count'],
              'entry certificates;', 'new:', p['new_entry_certificates'], 'lost:', p['lost_entry_certificates'])


if __name__ == '__main__':
    main()
    if '--checks' in sys.argv:
        record_checks()
