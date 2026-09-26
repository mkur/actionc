"""Publish the completed six slices and rank remaining operation footprints.

Run after measure.py has written all six summaries. Uses the same local frozen
probe output; footprints are emitted bytes, not forecasts of removable bytes.
"""
from pathlib import Path
import hashlib
import json
import sys

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'tools/compare65816'))
from build import image_guard_ranges

out=Path(__file__).parent
cache=ROOT/'target/exec-current-audit'
image_path=cache/'casts-final.image.json'
inventory_path=cache/'casts-final.inventory.json'
image=json.loads(image_path.read_text())
inventory=json.loads(inventory_path.read_text())
routines={r['id']:r for r in image['routines']}
segments={s['address']:s for s in image['segments'] if s['executable']}
rows=[]
for r in inventory['routines']:
    base=routines[r['id']]['address']
    guards=image_guard_ranges(image,segments[base])
    for s in r['spans']:
        start,end=base+s['start'],base+s['end']
        body=end-start-sum(max(0,min(end,b)-max(start,a)) for a,b in guards)
        if body: rows.append(s|dict(routine=r['name'],body=body))

families=[
    ('Calls, excluding guards',lambda k:k.startswith('Call/')),
    ('Unindexed indirect scalar loads/stores',lambda k:k.startswith(('Load/','Store/')) and k.endswith('/LongIndirect')),
    ('Remaining private 24-bit captures',lambda k:k in ['Load/3/Parameter','Load/3/AutomaticFrame']),
    ('BYTE equality/inequality comparisons',lambda k:k in ['Compare/1/false/Eq','Compare/1/false/Ne']),
    ('Explicit address formation',lambda k:k=='AddressOf/3'),
    ('Integer casts after native selection',lambda k:k.startswith('Cast/') and k.endswith('/Integer')),
]
rank=[]
for name,match in families:
    cohort=[s for s in rows if match(s['kind'])]
    rank.append(dict(pattern=name,sites=len(cohort),current_bytes=sum(s['body'] for s in cohort),estimated_saving=None))
rank.sort(key=lambda r:r['current_bytes'],reverse=True)
slices=[json.loads((out/d/'summary.json').read_text()) for d in ['01-void','02-value','03-byte','04-scaled','05-constant','06-casts']]
assert slices[0]['compiler_code_before']==354944
for a,b in zip(slices,slices[1:]): assert a['compiler_code_after']==b['compiler_code_before']
assert all(not s['larger_routines'] for s in slices)
saved=sum(s['saved'] for s in slices)
final=slices[-1]
record=dict(baseline_compiler='fc729cf0',workload='frozen Exec 622b139-dirty, 631 routines, 120 input hashes',
    compiler_code_before=354944,compiler_code_after=final['compiler_code_after'],saved=saved,
    slices=[dict(slice=s['after'],saved=s['saved']) for s in slices],
    guard_bytes=72252,guard_subtracted_compiler_code=final['guard_subtracted_compiler_code'],
    package_assembly=8300,total_initialized_data=2307,
    estimated_loaded_without_guards=final['estimated_loaded_without_guards'],release_cap=262144,
    remaining_gap=final['gap_to_256_kib'],model_saving=11678,
    qualification_run=False,guard_disabled_build=False,
    remaining_operation_footprints=rank,
    footprint_note='Disjoint operation families, excluding guard ranges; includes necessary computation and already optimized code. Not a savings estimate.',
    hashes={str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in [image_path,inventory_path,Path(__file__).resolve()]})
(out/'completed-summary.json').write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps({k:v for k,v in record.items() if k!='hashes'},indent=2))
