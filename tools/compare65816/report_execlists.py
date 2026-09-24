#!/usr/bin/env python3
"""Archive complete Exec list measurements, retaining incorrect compiler outputs."""
import argparse
import collections
import csv
import json
from pathlib import Path
import sys

sys.dont_write_bytecode = True
from build import ROOT, digest


def archive_listing(source, destination):
    # Host listing text has no byte-significant line endings or trailing padding.
    destination.write_text('\n'.join(line.rstrip() for line in source.read_text().splitlines())+'\n')


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--input',type=Path,default=ROOT/'target/execlists-comparison')
    p.add_argument('--output',type=Path,default=ROOT/'docs/benchmarks/65816-execlists')
    a=p.parse_args();src=a.input.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=True)
    load=lambda path:json.loads(path.read_text())
    debug=load(src/'debug.json');release=load(src/'release.json');manifest=load(src/'manifest.json')
    assert debug==release and debug['manifest']==manifest
    assert manifest['crlf_checked']
    for path,h in manifest['inputs'].items():assert digest(path)==h,path
    for tool in manifest['tools'].values():assert digest(tool['path'])==tool['sha256']
    for artifact in manifest['artifacts']:
        for path,h in artifact['hashes'].items():assert digest(Path(artifact['directory'])/path)==h
    records=debug['measurements']
    expected=sum(len(next(c for c in manifest['cases'] if c['id']==x['case'])['vectors']) for x in manifest['artifacts'])
    assert len(records)==expected
    key=lambda m:(m['compiler'],m['mode'],m['case'],m['vector'])
    assert len({key(m) for m in records})==expected
    assert all(m['correct'] for m in records if m['compiler']=='actionc')
    failures=[{k:m[k] for k in ['compiler','mode','case','vector','args','errors']} for m in records if not m['correct']]
    def table(name,rows):
        with (out/name).open('w',newline='') as f:
            w=csv.DictWriter(f,fieldnames=list(rows[0]),lineterminator='\n');w.writeheader();w.writerows(rows)
    table('sizes.csv',load(src/'sizes.json'))
    keys=['compiler','mode','case','vector','correct','code_bytes','cycles','stack_check_cycles','instructions','dp_reads','dp_writes','stack_reads','stack_writes','metadata_reads','input_padding_reads','peak_below_entry_s','static_stack_check_bytes']
    table('measurements.csv',[{k:m[k] for k in keys} for m in records])
    (out/'failures.json').write_text(json.dumps(failures,indent=2)+'\n')
    for compiler in sorted({x['compiler'] for x in manifest['artifacts']}):
        for mode in ['raw','optimized']:
            archive_listing(src/compiler/mode/'code.linked.lst',out/f'{compiler}-{mode}.lst')
    metadata=dict(compiler_revision=manifest['compiler_revision'],compiler_worktree_status=manifest['compiler_worktree_status'],
        tools=manifest['tools'],inputs=manifest['inputs'],crlf_checked=True,debug_release_identical=True,
        paired_mask_records=len(records),executions_per_host=2*len(records),
        correct_records=dict(collections.Counter(f'{m["compiler"]}/{m["mode"]}' for m in records if m['correct'])),
        failed_records=dict(collections.Counter(f'{m["compiler"]}/{m["mode"]}/{m["case"]}' for m in records if not m['correct'])),
        comparison_exit='Both cargo tests exit 101 for the retained incorrect vbcc results; the Python qualification wrapper exits 1.',
        observe_control_flow=manifest['observe_control_flow'],
        vm_base='56ddc5c5de41f0e7294e87c440869550eaf53292',
        vm_patch_sha256=digest(ROOT/'tools/native65816-runtime-tests/vm-status-timing.patch'),
        harness_sha256=digest(ROOT/'tools/native65816-runtime-tests/tests/code_quality.rs'),
        qualification_runner_sha256=digest(ROOT/'tools/native65816-runtime-tests/qualify.py'),
        report_tool_sha256=digest(Path(__file__)),
        results={p.name:digest(p) for p in [src/'manifest.json',src/'debug.json',src/'release.json',src/'debug.log',src/'release.log']},
        artifacts=[dict(compiler=x['compiler'],mode=x['mode'],hashes=x['hashes']) for x in manifest['artifacts'] if x['case']=='NewList'],
        report_artifacts={p.name:digest(p) for p in sorted(out.iterdir()) if p.is_file() and p.name!='provenance.json'})
    (out/'provenance.json').write_text(json.dumps(metadata,indent=2)+'\n')
    print(json.dumps({k:metadata[k] for k in ['paired_mask_records','correct_records','failed_records']},indent=2))


if __name__=='__main__':main()
