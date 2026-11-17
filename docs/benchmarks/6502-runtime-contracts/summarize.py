#!/usr/bin/env python3
"""Summarize a successful run.py, retaining the recorded starting baseline."""
import hashlib
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def read(path):
    return json.loads(path.read_text())


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def summarize(label, path, image):
    raw = read(path)
    rows = []
    for r in raw['routines']:
        rows.append({
            'name': r['name'], 'named_homes': r.get('storage') or [],
            'code_bytes': sum(len(line[6:14].split()) for line in (r.get('assembly') or '').splitlines()),
            'temporary_zp': r['temporary_zp'],
            'entry_available': r['acyclic_entry'] is not None, 'failure': r['acyclic_failure'],
            'entry': r['acyclic_entry'], 'direct_parameter_abi': r['direct_parameter_abi'],
            'direct_result_abi': r['direct_result_abi'],
            'helper_sites': [{k: h[k] for k in ['block', 'op', 'operation', 'available', 'reason', 'inputs', 'definitions', 'register_definitions', 'normal_writes']}
                             for h in r['helper_sites']],
            'library_sites': [{k: c[k] for k in ['block', 'op', 'library_binding', 'library_contract_available',
                              'library_inputs', 'library_definitions', 'library_buffers', 'site_returning_writes']}
                              for c in r['calls'] if c['library_binding'] is not None],
            'full_reads': r['reads'], 'full_writes': r['writes'],
            'normal_return_writes': r['normal_return_writes'],
        })
    return {'label': label, 'packaged_bytes': raw['packaged_bytes'], 'image_sha256': sha(image), 'routines': rows}


def stats(program):
    rows = program['routines']
    homes = {r['name']: [s for s in r['named_homes'] if s['address'] < 256] for r in rows}
    temps = {r['name']: r['temporary_zp'] for r in rows if r['temporary_zp']}
    return {'packaged_bytes': program['packaged_bytes'], 'image_sha256': program['image_sha256'],
            'named_zp_homes': {n: h for n, h in homes.items() if h},
            'named_zp_routine_count': sum(bool(h) for h in homes.values()),
            'temporary_zp': temps, 'temporary_zp_routine_count': len(temps),
            'entry_certificate_count': sum(r['entry_available'] for r in rows)}


def main():
    run = read(ROOT / 'build/runtime-contracts-qualification/run.json')
    assert run['return_code'] == 0 and run['hook_removed']
    baseline = read(HERE / 'baseline.json')
    current = {'TN': summarize('TN', ROOT / 'build/call-facts-audit-20261116/tn.json', ROOT / 'build/call-facts-audit-20261116/tn.xex')}
    for label in ['portable', 'driver', 'portable-no-inline']:
        current[label] = summarize(label, ROOT / f'build/huffman-facts/{label}.json', ROOT / f'build/huffman-facts/{label}.xex')
    receipt = {'production_revision': run['revision'], 'baseline_revision': baseline['revision'], 'run': run, 'programs': {}}
    for label, after in current.items():
        before = baseline[label]
        old = {r['name']: r for r in before['routines']}
        now = {r['name']: r for r in after['routines']}
        receipt['programs'][label] = {
            'before': stats(before), 'after': stats(after),
            'byte_delta': after['packaged_bytes'] - before['packaged_bytes'],
            'new_entry_certificates': [n for n,r in now.items() if r['entry_available'] and not old.get(n,{}).get('entry_available',False)],
            'changed_routine_code_sizes': [{'name':n,'before':old[n]['code_bytes'],'after':r['code_bytes']}
                                         for n,r in now.items() if n in old and old[n]['code_bytes'] != r['code_bytes']],
            'helper_sites': sum(len(r['helper_sites']) for r in after['routines']),
            'available_helper_sites': sum(h['available'] for r in after['routines'] for h in r['helper_sites']),
            'library_sites': sum(len(r['library_sites']) for r in after['routines']),
            'available_library_sites': sum(h['library_contract_available'] for r in after['routines'] for h in r['library_sites']),
        }
    receipt['TN_profile'] = read(ROOT / 'build/call-facts-audit-20261116/tn-profile.json')
    receipt['TN_configuration'] = {'source': 'samples/tn/modern/TN.ACT', 'origin': 0x3000,
                                   'runtime': 'ActionCart', 'config': 'Mir6502Config::optimized()'}
    receipt['TN_trace'] = read(ROOT / 'build/call-facts-audit-20261116/tn-contract-events.json')
    receipt['huffman_runs'] = {label: read(ROOT / f'build/huffman-facts/{label}-run.json') for label in current if label != 'TN'}
    receipt['huffman_events'] = {label: read(ROOT / f'build/huffman-facts/{label}-events.json') for label in current if label != 'TN'}
    sources = ['samples/tn/modern/TN.ACT', 'samples/tn/modern/LIB.ACT',
               'fixtures/runtime/tacle/huff_dec/huff_dec.act', 'fixtures/runtime/tacle/huff_dec/types.inc',
               'fixtures/runtime/tacle/huff_dec/kernel.inc', 'fixtures/runtime/tacle/huff_dec/vectors.txt',
               'tools/vm-runtime-tests/fixtures/huff_dec_driver.act']
    receipt['source_sha256'] = {p: sha(ROOT / p) for p in sources}
    assert receipt['source_sha256'] == baseline['source_sha256'], 'baseline sources changed'
    receipt['probe_sha256'] = {p: sha(ROOT / p) for p in [
        'docs/benchmarks/6502-call-facts-audit/probe.rs',
        'docs/benchmarks/6502-huffman-facts/probe.rs',
        'docs/benchmarks/6502-runtime-contracts/run.py',
        'docs/benchmarks/6502-runtime-contracts/summarize.py',
    ]}
    (HERE / 'facts.json').write_text(json.dumps(current, indent=2) + '\n')
    (HERE / 'qualification.json').write_text(json.dumps(receipt, indent=2) + '\n')
    for label, p in receipt['programs'].items():
        print(label, p['before']['packaged_bytes'], '->', p['after']['packaged_bytes'], 'bytes;', p['after']['named_zp_routine_count'], 'named ZP routines;', p['available_helper_sites'], '/', p['helper_sites'], 'helper sites')


if __name__ == '__main__':
    main()
