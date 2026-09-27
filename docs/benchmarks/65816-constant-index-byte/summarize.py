"""Combine the five independently checked frozen Exec deltas."""
from pathlib import Path
import json

out = Path(__file__).resolve().parent
slices = [json.loads((out / name / 'summary.json').read_text()) for name in
          ['01-constant', '02-zero', '03-byte', '04-y', '05-pea']]
assert slices[0]['compiler_code_before'] == 343783
for a, b in zip(slices, slices[1:]):
    assert a['compiler_code_after'] == b['compiler_code_before']
assert all(not s['larger_routines'] and s['same_frames_peaks_abi'] for s in slices)
final = slices[-1]
record = dict(
    baseline_compiler='a25fa91d',
    workload='frozen Exec 622b139-dirty, 631 routines, 120 checked input hashes',
    compiler_code_before=slices[0]['compiler_code_before'],
    compiler_code_after=final['compiler_code_after'],
    saved=sum(s['saved'] for s in slices),
    slices=[dict(slice=s['after'], saved=s['saved']) for s in slices],
    guard_bytes=final['guard_bytes'],
    guard_subtracted_compiler_code=final['guard_subtracted_compiler_code'],
    package_assembly=final['package_assembly'],
    all_initialized_data=final['all_initialized_data'],
    estimated_loaded_without_guards=final['estimated_loaded_without_guards'],
    release_cap=262144, remaining_gap=final['gap_to_256_kib'],
    full_qualification_run=False, guard_disabled_build=False,
    unchanged=['frozen inputs', 'ABI', 'frame reservations', 'local stack peaks',
               'guards', 'initialized data'],
)
assert record['saved'] == record['compiler_code_before'] - record['compiler_code_after']
(out / 'completed-summary.json').write_text(json.dumps(record, indent=2) + '\n')
print(json.dumps(record, indent=2))
