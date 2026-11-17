#!/usr/bin/env python3
"""Record the wide ABI comparison after the existing runtime-contract probe."""
import importlib.util
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
PREVIOUS = HERE.parent / '6502-runtime-contracts'
spec = importlib.util.spec_from_file_location('previous', PREVIOUS / 'summarize.py')
previous = importlib.util.module_from_spec(spec)
spec.loader.exec_module(previous)
read, sha, summarize, stats = previous.read, previous.sha, previous.summarize, previous.stats


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
    receipt = {'production_revision': run['revision'], 'baseline_revision': '7c8110aa',
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
        'docs/benchmarks/6502-wide-routine-abi/summarize.py',
    ]}
    (HERE / 'facts.json').write_text(json.dumps(current, indent=2) + '\n')
    (HERE / 'qualification.json').write_text(json.dumps(receipt, indent=2) + '\n')
    for label, p in receipt['programs'].items():
        print(label, p['before']['packaged_bytes'], '->', p['after']['packaged_bytes'], 'bytes;',
              p['before']['entry_certificate_count'], '->', p['after']['entry_certificate_count'],
              'entry certificates;', 'new:', p['new_entry_certificates'], 'lost:', p['lost_entry_certificates'])


if __name__ == '__main__':
    main()
