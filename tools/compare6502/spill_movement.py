#!/usr/bin/env python3
"""Audit rejected spill movement from complete captures; never authorize compaction."""
import argparse
import collections
import json
from pathlib import Path

from core_layout_census import sha


def root_key(home):
    return home['kind'], home['routine'], home['id']


def candidate_key(candidate):
    return candidate['family'], candidate['block'], candidate['start']


def audit(directory):
    manifest = json.loads((directory / 'manifest.json').read_text())
    assert manifest['status'] == manifest['core_layout_census']['status'] == 'complete'
    with (directory / 'events.jsonl').open() as stream:
        events = [json.loads(line) for line in stream]
    by_id = {e['id']: e for e in events}
    starts = {e['attempt']: e['data'] for e in events if e['kind'] == 'attempt-start'}
    returned = {e['attempt']: e for e in events if e['kind'] == 'snapshot' and e['data']['stage'] == 'attempt-return'}
    result = next(e for e in events if e['kind'] == 'late-spill-pruning-result')
    report = dict(bytes=manifest['output_sizes']['load_file_bytes'], work=manifest['work'], result=result['data'])
    movement = next((e for e in events if e['kind'] == 'core-allocation-movement'), None)
    if movement is None:
        assert result['data']['reason'] != 'CoreAllocation'
        return report
    delta = movement['data']['delta']
    assert delta['status'] == 'complete' and delta['omitted'] == 0
    assert delta['non_frame_facts_preserved']
    baseline = by_id[movement['data']['before_snapshot']]['data']
    proposed = by_id[movement['data']['proposed_snapshot']]['data']
    names = {r['id']: r['name'] for r in baseline['routines']}
    changes = {root_key(c['home']): c for c in delta['changes']}
    final_roots = {root_key(h) for f in baseline['frames'] for h in f['spills']}
    final_roots.update(root_key(s['home']) for f in baseline['frames'] for s in f['slots'] if not s['abi_only'])
    report['movement'] = delta
    report['routine_changes'] = [dict(routine=r, name=names[r], changed_homes=[c['home'] for c in delta['changes'] if c['home']['routine'] == r]) for r in delta['affected_routines']]
    assert {root_key(h) for f in baseline['frames'] for h in f['spills']} - {root_key(h) for f in proposed['frames'] for h in f['spills']} == {k for k,c in changes.items() if c['expected_removal']}

    def ancestor(attempt, stage):
        seen = set()
        while attempt in starts and attempt not in seen:
            seen.add(attempt)
            if starts[attempt]['stage'] == stage:
                return attempt
            attempt = starts[attempt]['parent']
        return None

    # Match the selected whole-program core result, not the latest trial or an
    # arbitrary applied local rewrite. Multiple equivalent suppliers stay ambiguous.
    roots = [a for a,s in starts.items() if s['parent'] == 0 and s['stage'] == 'inlining' and a in returned]
    suppliers = []
    if len(roots) == 1:
        selected_identity = returned[roots[0]]['data']['program_identity']
        suppliers = [a for a,s in starts.items() if s['stage'] == 'core-with-helpers' and a in returned and returned[a]['data']['program_identity'] == selected_identity and ancestor(a, 'inlining') == roots[0]]
    cores = [a for a,s in starts.items() if s['stage'] == 'core-materialization' and ancestor(a, 'core-with-helpers') in suppliers]
    selected = cores[0] if len(suppliers) == len(cores) == 1 else None
    report['selected_core'] = dict(status='unique-equivalent-supplier' if selected is not None else 'unknown-or-ambiguous', helper_attempts=suppliers, core_attempts=cores,
                                  coverage='Selected core image provenance only; not final instruction survival or a proof of dependency.')
    remaps = [e for e in events if e['kind'] == 'home-remap' and ancestor(e['attempt'], 'core-materialization') == selected] if selected is not None else []

    def footprint(item, event, outside):
        home = dict(item['home'])
        path = []
        if home['kind'] == 'spill':
            if outside:
                return 'unknown-unscoped-spill-epoch', home, path
            for remap in remaps:
                if remap['id'] <= event['id']:
                    continue
                matches = [m for m in remap['data']['entries'] if m['from'] == home]
                if len(matches) > 1:
                    return 'unknown-ambiguous-remap', home, path
                if matches:
                    home = dict(matches[0]['to'])
                    path.append(remap['id'])
        change = changes.get(root_key(home))
        if change:
            if change['expected_removal']:
                return 'erased-proof-home', home, path
            width = item['bytes'] or 1
            if home['offset'] + width > change['before']['bytes']:
                return 'unknown-range-outside-root', home, path
            return 'moved-survivor', home, path
        if home['kind'] in ['global', 'static', 'absolute', 'fixed-zp']:
            return 'unmoved-non-frame-input', home, path
        if root_key(home) in final_roots:
            return 'unmoved-frame-input', home, path
        return 'unknown-or-retired-home', home, path

    counts = collections.defaultdict(collections.Counter)
    examples = {}
    affected_rounds = []
    other_core_rounds = 0
    for event in events:
        outside = event['kind'] == 'core-layout-unattributed'
        if event['kind'] != 'core-layout-round' and not outside:
            continue
        owner = ancestor(event['attempt'], 'core-materialization')
        if selected is None or owner != selected:
            other_core_rounds += not outside
            continue
        data = event['data']
        candidates = {candidate_key(c['key']): c for c in data.get('candidates', [])}
        affected = collections.Counter()
        for entry in data['observations']:
            observation = entry['observation']
            local = observation.get('candidate')
            if local:
                scope = local['family'] + '/' + candidates[candidate_key(local)]['outcome']
            else:
                scope = 'outside-driver' if outside else 'unscoped/applied-round' if data['applied_count'] else 'unscoped/no-applied-plan'
            for item in observation['inputs']:
                kind, home, path = footprint(item, event, outside)
                label = scope + '/' + observation['predicate']
                counts[label][kind] += entry['evaluations']
                if kind in ['moved-survivor', 'erased-proof-home']:
                    affected[kind] += entry['evaluations']
                    examples.setdefault((data.get('routine'), label, kind), dict(event=event['id'], attempt=event['attempt'], generation=data.get('generation'), routine=data.get('routine'), routine_name=names.get(data.get('routine')), scope=scope,
                        applied=data.get('applied'), observation=observation, mapped_home=home, remap_events=path))
        if affected and not outside and data['applied_count']:
            affected_rounds.append(dict(event=event['id'], attempt=event['attempt'], generation=data['generation'], routine=data['routine'], routine_name=names.get(data['routine']), applied=data['applied'], affected_input_evaluations=dict(affected)))
    report['selected_core_input_evaluations'] = {k:dict(v) for k,v in sorted(counts.items())}
    report['selected_core_applied_rounds_with_affected_inputs'] = affected_rounds
    # Exact event references retain reproducibility without copying the full
    # observation and mixed-round plan payload into every corpus summary.
    report['examples'] = [dict(event=e['event'], attempt=e['attempt'],
        generation=e['generation'], routine=e['routine'], routine_name=e['routine_name'],
        scope=e['scope'], predicate=e['observation']['predicate'],
        mapped_home=e['mapped_home'], remap_events=e['remap_events']) for e in examples.values()]
    report['other_or_unresolved_core_rounds'] = other_core_rounds
    report['selected_late_families'] = [dict(attempt=e['attempt'], **e['data']) for e in events if e['kind'] == 'selection' and starts[e['attempt']]['parent'] == 0]
    report['coverage_gaps'] = [
        'Input evaluations can include rejected candidates and shared analysis. Unscoped applied-round inputs are not per-rewrite obligations.',
        'Observed epoch addresses differ from terminal planning addresses. No recorded predicate is replayed by this audit.',
        'Exact recorded spill remaps are followed only within the unique selected core. Unscoped spill timing, retired homes and absent mappings remain unknown.',
        'No persistent operation identity through later rewrites, complete inherited-fact provenance, or coverage of uninstrumented layout owners.',
        'The core guard rejects before candidate emission: physical movement, exposure safety, late-proof replay and final cost are not tested by this rejection.',
    ]
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline-api', required=True, type=Path)
    parser.add_argument('--current-api', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    report = dict(baseline_revision='41fc7276', cases=[])
    directories = sorted(p for p in args.current_api.iterdir() if p.is_dir())
    assert len(directories) == 12
    for current in directories:
        baseline = args.baseline_api/current.name
        for artifact in ['output.xex', 'output.debug']:
            assert (baseline/artifact).read_bytes() == (current/artifact).read_bytes(), (current.name, artifact)
        before = json.loads((baseline/'manifest.json').read_text())
        assert before['status'] == before['core_layout_census']['status'] == 'complete'
        result = audit(current)
        assert before['work'] == result['work']
        report['cases'].append(dict(id=current.name, full_output_sha256=sha((current/'output.debug').read_bytes()), image_sha256=sha((current/'output.xex').read_bytes()), **result))
        print(current.name, result['result'], 'moved:', result.get('movement',{}).get('moved_survivors'), flush=True)
    args.output.write_text(json.dumps(report, indent=2)+'\n')


if __name__ == '__main__':
    main()
