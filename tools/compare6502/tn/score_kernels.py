#!/usr/bin/env python3
"""Summarize qualified callable-control kernels, keeping ABI scope explicit."""
import argparse
import json
from pathlib import Path
import re
from build import ROOT,OUT,digest

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('log',type=Path);args=p.parse_args();args.log=args.log.resolve()
    listing=(OUT/'reference/TN-standalone.asm').read_text().replace('\r\n','\n')
    metrics={}
    for line in args.log.read_text().splitlines():
        m=re.fullmatch(r'tn-c-kernel,(\w+),(\w+),([0-9A-F]+),(\d+)',line)
        if m:
            key=(m[2],m[3]);row=metrics.setdefault(key,{})
            value=int(m[4]);assert m[1] not in row or row[m[1]]==value
            row[m[1]]=value
    rows=[]
    for name in ['Strcpy','Strcat','Instr','Fnamecmp','Sort','Convert','Print','PrintFname']:
        m=re.search(r'; ===== PROC '+name+r' \$([0-9A-F]+)\.\.\$([0-9A-F]+)',listing)
        sizes={'Action':int(m[2],16)-int(m[1],16)}
        for profile in ['Os','O3']:
            memory=json.loads((OUT/f'kernels-{profile}.dbj').read_text())['memory']
            obj=next(r for r in memory if r['name']==name);sizes[profile]=obj['end']-obj['start']
        cases=[dict(case=k[1],cycles=v) for k,v in metrics.items() if k[0]==name]
        matched=[c for c in cases if all(l in c['cycles'] for l in ['Action','Os','O3'])]
        samples=[]
        if matched:
            for sample in [min(matched,key=lambda c:c['cycles']['Action']),max(matched,key=lambda c:c['cycles']['Action'])]:
                if sample not in samples:samples.append(sample)
        rows.append(dict(routine=name,callee_bytes=sizes,matched_cases=len(matched),sample_callee_cycles=samples))
    result=dict(scope='Diagnostic C TN_DIAGNOSTIC callable/no-inline controls versus intact standalone Action! TN; not whole-program production sizes. Callee cycles include nested callees and return, exclude caller argument placement and harness. ABI and stronger C automatic-storage guarantees remain part of these results.',
                input_manifest='baseline.json',raw_metric_log=str(args.log.relative_to(ROOT)),raw_metric_sha256=digest(args.log),
                artifacts={f'kernels-{p}.xex':digest(OUT/f'kernels-{p}.xex') for p in ['Os','O3']},routines=rows)
    (ROOT/'surveys/tn/c-oscar64/kernels.json').write_text(json.dumps(result,indent=2)+'\n')
    for row in rows:print(row['routine'],row['callee_bytes'],row['sample_callee_cycles'])
if __name__=='__main__':main()
