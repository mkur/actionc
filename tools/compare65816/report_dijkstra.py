#!/usr/bin/env python3
"""Archive authenticated Dijkstra measurements and listings; never rank failed runs."""
import argparse
import csv
import json
from pathlib import Path
import shutil
import sys

sys.dont_write_bytecode = True
from build import ROOT, digest
from run_dijkstra import verify

FIELDS = ['compiler','mode','case','cycles','instructions','peak_below_entry_s',
          'stack_reads','stack_writes','dp_reads','dp_writes','metadata_reads',
          'mode_switches','stack_check_cycles','queue_stride']


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest',type=Path)
    parser.add_argument('results',type=Path)
    parser.add_argument('--qualification',type=Path,required=True)
    parser.add_argument('--output',type=Path,default=ROOT/'docs/benchmarks/65816-dijkstra')
    args=parser.parse_args()
    manifest=json.loads(args.manifest.read_text());verify(manifest)
    results=json.loads(args.results.read_text())
    assert not results['filter'], 'archive the complete comparison'
    results=results['results']
    labels=[l.split()[0] for l in Path(manifest['vectors']).read_text().splitlines() if l and not l.startswith('#')]
    expected={(c,m,label) for c in ('actionc','vbcc') for m in ('raw','optimized') for label in labels}
    assert len(results)==len(expected)==132
    assert {(r['compiler'],r['mode'],r['case']) for r in results}==expected
    assert all(not r['errors'] and r['interrupt_masks']==[0,4] for r in results)
    qualification=json.loads(args.qualification.read_text())
    # Accept either qualify.py's manifest or the authenticated runner's wrapper.
    if 'vm_qualification' in qualification:
        assert qualification['manifest_sha256']==digest(args.manifest)
        assert qualification['results_sha256']==digest(args.results)
        qualification=qualification['vm_qualification']
    for name,expected_hash in qualification['compiler_and_fixture_inputs'].items():
        assert digest(ROOT/name)==expected_hash, f'Changed qualified source: {name}'
    args.output.mkdir(parents=True,exist_ok=True)
    with (args.output/'results.csv').open('w',newline='') as output:
        writer=csv.DictWriter(output,fieldnames=FIELDS,extrasaction='ignore',lineterminator='\n')
        writer.writeheader();writer.writerows(results)
    with (args.output/'routine-profile.csv').open('w',newline='') as output:
        columns=['compiler','mode','case','routine','cycles','instructions','stack_reads','stack_writes','dp_reads','dp_writes','mode_switches','stack_check_cycles']
        writer=csv.DictWriter(output,fieldnames=columns,extrasaction='ignore',lineterminator='\n');writer.writeheader()
        for r in results:
            if r['case'] in ('benchmark-original','original-0-50'):
                for name,counters in r['routines'].items():
                    writer.writerow(dict(compiler=r['compiler'],mode=r['mode'],case=r['case'],routine=name,**counters))
    profiles=[{k:v for k,v in r.items() if k in ('compiler','mode','case','instruction_sites')} for r in results if 'instruction_sites' in r]
    (args.output/'instruction-profile.json').write_text(json.dumps(profiles,indent=2)+'\n')
    for a in manifest['artifacts']:
        shutil.copyfile(Path(a['directory'])/'code.linked.lst',args.output/f'{a["compiler"]}.{a["mode"]}.lst')
    facts=dict(manifest=manifest, results_sha256=digest(args.results),
               qualification=qualification, full_state_passes=264)
    # Keep evidence readable and relocatable, not host-specific absolute paths.
    serialized=json.dumps(facts,indent=2).replace(str(ROOT),'<repo>').replace(str(Path.home()),'<home>')
    (args.output/'provenance.json').write_text(serialized+'\n')
    for r in results:
        if r['case']=='benchmark-original':
            a=next(a for a in manifest['artifacts'] if a['compiler']==r['compiler'] and a['mode']==r['mode'])
            print(f"{r['compiler']:7} {r['mode']:9} code={a['code_bytes']:,} cycles={r['cycles']:,} stack={r['peak_below_entry_s']} stack-R/W={r['stack_reads']+r['stack_writes']:,} DP-R/W={r['dp_reads']+r['dp_writes']:,}")


if __name__=='__main__':
    main()
