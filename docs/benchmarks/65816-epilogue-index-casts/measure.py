"""Compare two frozen Exec inventory probe outputs; no hosted qualification.

Usage: python3 -B docs/benchmarks/65816-epilogue-index-casts/measure.py BEFORE AFTER SLICE
The stems refer to target/exec-current-audit/{stem}.{image,inventory}.json.
"""
from pathlib import Path
import csv
import hashlib
import json
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'tools/compare65816'))
from build import image_guard_ranges


def load(path):
    return json.loads(path.read_text())


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def measure(before, after, slice_name):
    cache = ROOT / 'target/exec-current-audit'
    output = Path(__file__).parent / slice_name
    output.mkdir(exist_ok=True)
    images = [load(cache / (stem + '.image.json')) for stem in [before, after]]
    inventories = [load(cache / (stem + '.inventory.json')) for stem in [before, after]]
    frozen = load(cache / 'frozen-inputs.json')
    for item in frozen:
        assert sha(cache / 'frozen-exec' / item['path']) == item['snapshot_sha256'], item['path']
    a, b = images
    for key in ['zero_fill', 'data', 'imports', 'task_headroom', 'irq_headroom']:
        assert a[key] == b[key], key
    assert len(a['routines']) == len(b['routines']) == 631
    rows, spans, guards, added_jumps = [], [], [], 0
    for x, y, m, n in zip(a['routines'], b['routines'], inventories[0]['routines'], inventories[1]['routines']):
        assert x['name'] == y['name'] == m['name'] == n['name']
        assert {k:v for k,v in x.items() if k not in ['address','size']} == {k:v for k,v in y.items() if k not in ['address','size']}, x['name']
        rows.append(dict(routine=x['name'], before=x['size'], after=y['size'], saved=x['size']-y['size']))
        assert len(m['spans']) == len(n['spans'])
        for u, v in zip(m['spans'], n['spans']):
            assert {k:z for k,z in u.items() if k not in ['start','end']} == {k:z for k,z in v.items() if k not in ['start','end']}, x['name']
            old, new = u['end']-u['start'], v['end']-v['start']
            if old != new:
                spans.append(dict(routine=x['name'], block=u['block'], index=u['index'], kind=u['kind'], before=old, after=new, saved=old-new))
        # Shared tails add typed local jumps and shift label identities. Require
        # all new targets to be bound and all remapped instruction ranges valid.
        added_jumps += len(n['jumps'])-len(m['jumps'])
        labels = n['labels']
        for jump in n['jumps']:
            target = labels[str(jump['target'])] if isinstance(labels, dict) else labels[jump['target']]
            assert target is not None
            assert 0 <= jump['offset'] < jump['offset'] + jump['size'] <= y['size']
        pair = []
        for image, routine in [(a,x),(b,y)]:
            segment = next(s for s in image['segments'] if s['address'] == routine['address'])
            pair.append([(hi-lo, bytes(segment['bytes'][lo-routine['address']:hi-routine['address']-4]))
                         for lo,hi in image_guard_ranges(image,segment)])
        assert pair[0] == pair[1], ('guard shape/amount',x['name'])
        guards.extend(length for length,_ in pair[1])
    assert len(guards) == 2676 and sum(guards) == 72252
    for name, values in [('routines.csv',rows),('spans.csv',spans)]:
        with (output/name).open('w', newline='') as f:
            writer=csv.DictWriter(f, fieldnames=list(values[0]), lineterminator='\n')
            writer.writeheader(); writer.writerows(values)
    code=sum(r['size'] for r in b['routines'])
    summary=dict(before=before, after=after, compiler_code_before=sum(r['size'] for r in a['routines']),
                 compiler_code_after=code, saved=sum(r['saved'] for r in rows),
                 guard_bytes=sum(guards), guards=len(guards), guard_subtracted_compiler_code=code-sum(guards),
                 package_assembly=8300, all_initialized_data=2307,
                 estimated_loaded_without_guards=code-sum(guards)+8300+2307,
                 gap_to_256_kib=code-sum(guards)+8300+2307-262144,
                 smaller_routines=sum(r['saved']>0 for r in rows), larger_routines=[r for r in rows if r['saved']<0],
                 added_local_jumps=added_jumps, same_mir_operations=True, same_frames_peaks_abi=True,
                 frozen_input_files=len(frozen), full_qualification_run=False, guard_disabled_build=False,
                 hashes={str(p.relative_to(ROOT)):sha(p) for p in [cache/'frozen-inputs.json',
                     *(cache/(stem+'.'+kind+'.json') for stem in [before,after] for kind in ['image','inventory']),Path(__file__).resolve()]})
    assert sum(len(s['bytes']) for s in b['segments'] if not s['executable']) == 951
    (output/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
    print(json.dumps({k:v for k,v in summary.items() if k!='hashes'},indent=2))


if __name__ == '__main__':
    measure(*sys.argv[1:])
