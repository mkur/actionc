#!/usr/bin/env python3
"""Snapshot checked measurements, provenance, and representative final listings.

Compiler correctness failures remain visible in the CSV and tables. They are
never silently removed, nor included in valid-code performance comparisons.
"""
import argparse
import csv
import hashlib
import io
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PICKS = dict(identity=2, add=2, subtract=2, constant_chain=2, maximum=2, wide_shift=2,
             loop_rotation=2, sum_loop=3, recursive_sum=3, direct_calls=2,
             byte_sum=4, record_field=1, unlink=0, forward_copy=0)


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', type=Path, default=ROOT/'target/code-quality-65816')
    parser.add_argument('--output', type=Path, default=ROOT/'target/code-quality-65816/snapshot')
    args = parser.parse_args()
    directory = args.input.resolve()
    manifest = json.loads((directory/'manifest.json').read_text())
    debug = json.loads((directory/'debug.json').read_text())
    release = json.loads((directory/'release.json').read_text())
    assert debug == release, 'debug/release measurements differ'
    assert debug['manifest'] == manifest, 'measurements describe an older build'
    for relative, sha256 in manifest['inputs'].items():
        assert digest(ROOT/relative) == sha256, f'changed build input: {relative}'
    measured = debug['measurements']
    expected = {(c['id'], mode, compiler, i)
                for c in manifest['cases'] for i in range(len(c['vectors']))
                for mode in ('raw', 'optimized') for compiler in ('actionc', 'vbcc')}
    indexed = {(r['case'], r['mode'], r['compiler'], r['vector']): r for r in measured}
    assert len(indexed) == len(measured) and set(indexed) == expected
    for artifact in manifest['artifacts']:
        for name, sha256 in artifact['hashes'].items():
            assert digest(Path(artifact['directory'])/name) == sha256, f'changed artifact: {artifact}/{name}'
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    content = io.StringIO(newline='')
    writer = csv.DictWriter(content, fieldnames=list(measured[0]), lineterminator='\n')
    writer.writeheader()
    for row in measured:
        writer.writerow({k: json.dumps(v, separators=(',', ':')) if isinstance(v, (list, bool)) else v
                         for k, v in row.items()})
    (output/'results.csv').write_text(content.getvalue())
    lines = ['# Measured 65816 comparison', '',
             'Each cell is **actionc / vbcc**. Code includes worker/helper bodies, checked',
             'entries and RTL. Cycles run from worker entry through RTL. Stack is the',
             'observed peak below entry S; caller-owned arguments and return bytes are',
             'reported separately in `results.csv`. `INVALID` means wrong memory output.', '']
    for mode in ('optimized', 'raw'):
        lines.extend([f'## {mode.title()}', '',
                      '| Kernel | Runtime arguments | Code bytes | Cycles | Stack bytes |',
                      '| --- | --- | ---: | ---: | ---: |'])
        for c in manifest['cases']:
            name = c['id']
            pair = [indexed[name, mode, compiler, PICKS[name]] for compiler in ('actionc', 'vbcc')]
            values = [f'${v:06X}' if size > 2 else str(v)
                      for size, v in zip(c['args'], pair[0]['args'])]
            cells = []
            for field in ('code_bytes', 'cycles', 'peak_below_entry_s'):
                cells.append(' / '.join(str(r[field]) if r['correct'] else f"INVALID ({r[field]})" for r in pair))
            lines.append('| '+' | '.join([name, ', '.join(values), *cells])+' |')
        lines.append('')
    (output/'tables.md').write_text('\n'.join(lines))
    inputs = dict(manifest['inputs'])
    for path in [Path(__file__), ROOT/'tools/native65816-runtime-tests/tests/code_quality.rs',
                 ROOT/'tools/native65816-runtime-tests/tests/support/mod.rs',
                 ROOT/'tools/native65816-runtime-tests/Cargo.toml',
                 ROOT/'tools/native65816-runtime-tests/Cargo.lock',
                 ROOT/'tools/native65816-runtime-tests/qualify.py',
                 ROOT/'tools/disassemble65816.py']:
        inputs[str(path.relative_to(ROOT))] = digest(path)
    artifacts = []
    for a in manifest['artifacts']:
        entry = {k: a[k] for k in ('case', 'compiler', 'mode', 'code_bytes', 'static_stack_check_bytes', 'hashes')}
        # Linker/assembler logs contain absolute local paths. Binary hashes and
        # final listings are portable; retain full logs in the ignored build dir.
        entry['hashes'] = {k: v for k, v in entry['hashes'].items() if k != 'code.lst'}
        artifacts.append(entry)
        if a['case'] in ('identity', 'add', 'subtract', 'maximum', 'sum_loop', 'loop_rotation', 'byte_sum', 'unlink'):
            source = Path(a['directory'])/('code.asm' if a['compiler'] == 'actionc' else 'code.linked.lst')
            (output/f"{a['case']}.{a['mode']}.{a['compiler']}.lst").write_text(source.read_text())
    provenance = dict(
        target=manifest['target'], compiler_revision=manifest['compiler_revision'],
        compiler_worktree_status=manifest['compiler_worktree_status'],
        versions=manifest['version_banners'],
        tools={k: dict(filename=Path(v['path']).name, sha256=v['sha256']) for k, v in manifest['tools'].items()},
        vm_base='56ddc5c5de41f0e7294e87c440869550eaf53292',
        vm_patch_sha256=digest(ROOT/'tools/native65816-runtime-tests/vm-status-timing.patch'),
        crlf_identical=manifest['crlf_checked'], debug_release_identical=True,
        paired_mask_records=len(measured), executions_per_host_build=2*len(measured),
        incorrect=[{k: r[k] for k in ('case', 'mode', 'compiler', 'vector', 'errors')}
                   for r in measured if not r['correct']],
        inputs_sha256=inputs, artifacts=artifacts,
        results_sha256=digest(output/'results.csv'),
        settings=dict(actionc_raw=['--no-opt'], actionc_optimized=[],
                      vbcc_raw=['-O=0'], vbcc_optimized=['-O=1023'],
                      vbcc_common=['-mhuge', '-ptr24', '-near-threshold=0', '-no-near-const'],
                      vasm=['-816', '-opt-branch', '-Fvobj'], vlink=['-brawbin1'],
                      code_origin=0x10000, entry_s=0x5fe0, direct_page=0x2000,
                      interrupt_masks=[0, 4], irq_nmi_injected=False))
    (output/'provenance.json').write_text(json.dumps(provenance, indent=2)+'\n')
    print(f'Saved {len(measured)} records; {len(provenance["incorrect"])} incorrect outputs retained: {output}')


if __name__ == '__main__':
    main()
