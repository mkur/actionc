#!/usr/bin/env python3
"""Collect the pinned audit artifacts; --check compares maintained measurements."""
import argparse
import csv
import hashlib
import io
import json
from pathlib import Path
import runpy

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def csv_text(header, rows):
    out = io.StringIO(newline="")
    writer = csv.writer(out, lineterminator="\n")
    writer.writerow(header)
    writer.writerows(rows)
    return out.getvalue()


def profile(path):
    text = path.read_text()
    assert "test result: ok. 3 passed; 0 failed" in text, path
    totals, routines = {}, {}
    for line in text.splitlines():
        parts = line.split(',')
        if len(parts) == 5 and parts[0] in {"Optimized", "Mir6502"}:
            key = tuple(parts[:3])
            value = [int(v) for v in parts[3:]]
            assert key not in totals or totals[key] == value
            totals[key] = value
        elif len(parts) == 14 and parts[0] == "tn-routine" and parts[2] in {"Optimized", "Mir6502"}:
            key = tuple(parts[1:6])
            value = [int(v) for v in parts[6:]]
            assert key not in routines or routines[key] == value
            routines[key] = value
    for (mode, workload, count), (cycles, spills) in totals.items():
        selected = [v for k,v in routines.items() if k[1:4] == (mode,workload,count)]
        assert sum(v[0] for v in selected) == cycles
        assert sum(v[5] for v in selected) == spills
    return totals, routines


def generate(art):
    parse = runpy.run_path(str(ROOT / "surveys/tn/audit-current-output.py"))["census"]
    census = {}
    for variant in ["baseline", "masks", "predicate", "combined"]:
        folder = art if variant == "baseline" else art / variant
        data = json.loads((folder / "census.json").read_text())
        for stem, observed in data.items():
            listing = (folder / (stem + ".asm")).read_text()
            blob = (folder / (stem + ".xex")).read_bytes()
            assert parse(listing, blob) == observed
            assert parse(listing.replace('\n', '\r\n'), blob) == observed
        census[variant] = data
    qualified = json.loads((ROOT / "surveys/tn/size-reduction/final.json").read_text())
    assert sha(ROOT / "target/tn-size-reduction/final/actionc-qualified") == qualified['compiler_sha256']
    for image in qualified["images"]:
        stem = Path(image["program"]).stem + '-' + image["mode"]
        assert census["baseline"][stem]["image_sha256"] == image["sha256"]

    static_rows = []
    for stem, data in census["baseline"].items():
        for name, r in sorted(data["routines"].items()):
            static_rows.append([stem, name, f"${r['entry']:04X}", r['instruction_form_bytes'],
                               r['directive_form_bytes'], *r['home_bytes'].values(),
                               *r['fixed_home_bytes'].values(), r['direct_calls'],
                               r['pointer_scratch_writes'], r['indirect_y_operations'],
                               r['abi_cell_accesses'], *r['static_home_operand_accesses'].values()])
    static_header = ['program_backend','routine','entry','instruction_form_bytes','directive_form_bytes',
                     'emitted_spill_bytes','emitted_param_bytes','emitted_local_bytes','string_bytes',
                     'fixed_spill_bytes','fixed_param_bytes','fixed_local_bytes','fixed_string_bytes',
                     'jsr_sites','pointer_scratch_write_sites','indirect_y_sites','abi_cell_operands',
                     'spill_operands','param_operands','local_operands','string_operands']

    profile_rows, total_rows = [], []
    for variant, program, filename in [
        ('baseline','TN','TN-profile.log'), ('baseline','TNDBG','TNDBG-profile.log'),
        ('masks','TN','masks-profile.log'), ('predicate','TN','predicate-profile.log'),
        ('combined','TN','combined-profile.log'), ('combined','TNDBG','combined-TNDBG-profile.log')]:
        totals, routines = profile(art / filename)
        total_rows.extend([variant,program,*key,*value] for key,value in sorted(totals.items()))
        profile_rows.extend([variant,*key,*value] for key,value in sorted(routines.items()))
    repro_rows = []
    text = (art / 'reproducers-vm.log').read_text()
    assert 'test result: ok. 3 passed; 0 failed' in text
    for line in text.splitlines():
        parts = line.split(',')
        if len(parts) == 7 and parts[0] == 'audit-reproducer' and parts[1] in {'Optimized','Mir6502'}:
            repro_rows.append(parts[1:])
    repro_static = {}
    for name in ['escaping-record-address','captured-call-arguments','bounded-bit-mask']:
        data = parse((art / (name+'.asm')).read_text(), (art / (name+'.xex')).read_bytes())
        repro_static[name] = {n:dict(routine_bytes=r['end']-r['start'],
                                    emitted_homes=r['home_bytes'], calls=r['calls'])
                             for n,r in data['routines'].items() if n!='Main'}

    memory = {}
    for variant, data in census.items():
        memory[variant] = {}
        for stem, value in data.items():
            memory[variant][stem] = {k:v for k,v in value.items() if k!='routines'}
            memory[variant][stem]['load_bytes_saved'] = census['baseline'][stem]['load_file_bytes']-value['load_file_bytes']
    selected = {}
    for stem in ['TN-mir6502','TNDBG-mir6502']:
        routines = census['baseline'][stem]['routines']
        selected[stem] = {n:r for n,r in sorted(routines.items(),key=lambda pair:-pair[1]['instruction_form_bytes'])[:15]}
    gaps = []
    mir = census['baseline']['TN-mir6502']['routines']
    classic = census['baseline']['TN-optimized']['routines']
    for name in sorted(mir.keys() & classic.keys()):
        delta = mir[name]['instruction_form_bytes']-classic[name]['instruction_form_bytes']
        gaps.append(dict(routine=name, mir_bytes=mir[name]['instruction_form_bytes'],
                         classic_bytes=classic[name]['instruction_form_bytes'], delta=delta))
    report = dict(date='2026-10-12', audited_revision='02d79b01', compiler_source_revision='1d67c984',
                  compiler_sha256=qualified['compiler_sha256'],
                  emitter_sha256=sha(ROOT/'target/debug/actionc-emit'),
                  profiler_source_sha256=sha(ROOT/'tools/vm-runtime-tests/tests/tn_size_profile.rs'),
                  census_source_sha256=sha(ROOT/'surveys/tn/audit-current-output.py'),
                  source_sha256={str(p.relative_to(ROOT)):sha(p) for p in sorted((ROOT/'samples/tn/modern').glob('*.ACT'))},
                  reproducer_sha256={n:sha(ROOT/'surveys/tn/reproducers'/f'{n}.act') for n in repro_static},
                  runtime='cart', origin='$2C00', memory=memory,
                  largest_mir_routines=selected, largest_mir_classic_gaps=sorted(gaps,key=lambda r:(-r['delta'],r['routine']))[:12],
                  reproducers=repro_static,
                  qualification=dict(full_root_byte_reconciliation=4, baseline_images_match_qualification=True,
                                     full_listing_lf_crlf_census_equality=True, profile_cycle_and_spill_reconciliation=True,
                                     generic_modes=['optimized','mir6502'], generic_runtimes=['cart','standalone'],
                                     generic_actual_lf_crlf_compilation=True, generic_guard_checks=True,
                                     production_sources_changed=False),
                  scope='Static counts are MADS instruction/directive forms; classic storage is often inside PROC ranges and MIR homes outside. Fixed named cells are separate from emitted homes. Named storage categories can alias and are not additive. Executed operands exclude pointer-fetch bytes, stack and resident-service traffic; cycles include executed ROM helpers and the wrapper but screen/disk services are substituted. Copy and Dbg have static census only. Experiments are diagnostic counterfactuals, not release-qualified production changes.')
    return {'measurements.json':json.dumps(report,indent=2)+'\n',
            'routines.csv':csv_text(static_header,static_rows),
            'profiles.csv':csv_text(['experiment','program','backend','workload','count','routine','exclusive_cycles',
                                    'instructions','jsr','pointer_scratch_writes','indirect_y','spill_operand_accesses',
                                    'param_operand_accesses','local_operand_accesses'],profile_rows),
            'workloads.csv':csv_text(['experiment','program','backend','workload','count','cycles','spill_operand_accesses'],total_rows),
            'reproducers.csv':csv_text(['backend','runtime','crlf','routine','index_or_setting','cycles_including_wrapper'],repro_rows)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifacts',type=Path,default=ROOT/'target/tn-output-audit')
    parser.add_argument('--check',action='store_true')
    args = parser.parse_args()
    for name, content in generate(args.artifacts).items():
        if args.check:
            assert (HERE/name).read_text() == content, name
        else:
            (HERE/name).write_text(content)
    print('Audit reports checked' if args.check else 'Audit reports written')


if __name__ == '__main__':
    main()
