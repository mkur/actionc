#!/usr/bin/env python3
"""Inventory typed edge copies against qualified final bytes and saved VM counts."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import sys

sys.dont_write_bytecode = True
from check_empty_edges import listing
from delta import load

ROOT = Path(__file__).resolve().parents[2]


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def overlap(a, b):
    return (a['kind'] == b['kind'] and a['kind'] in ('stack', 'dp')
            and max(a['offset'], b['offset']) <
            min(a['offset'] + a['width'], b['offset'] + b['width']))


def classify(moves):
    """j must consume its old source before i writes an overlapping destination."""
    dependencies = [(j, i) for i, m in enumerate(moves) for j, other in enumerate(moves)
                    if i != j and overlap(m['destination'], other['source'])]
    self_copies = [i for i, m in enumerate(moves)
                   if overlap(m['source'], m['destination'])
                   and m['source']['offset'] == m['destination']['offset']
                   and m['source']['width'] == m['destination']['width']]
    partial = [(i, j) for i, m in enumerate(moves) for j, other in enumerate(moves)
               if overlap(m['destination'], other['source'])
               and (m['destination']['offset'], m['destination']['width']) !=
               (other['source']['offset'], other['source']['width'])]
    repeated = [(i, j) for i, m in enumerate(moves) for j, other in enumerate(moves[:i])
                if m['source']['kind'] in ('stack', 'dp') and
                all(m['source'][k] == other['source'].get(k) for k in ('kind', 'offset', 'width'))]
    pending = set(range(len(moves)))
    order = []
    while pending:
        ready = [i for i in sorted(pending) if not any(b == i and a in pending for a, b in dependencies)]
        if not ready:
            break
        # Retain the original last assignment as last when dependency order allows it.
        i = next((i for i in ready if i != len(moves)-1), ready[0])
        order.append(i)
        pending.remove(i)
    kind = ('empty' if not moves else 'cyclic' if pending else
            'independent' if not dependencies else
            'ordered_acyclic' if all(a < b for a, b in dependencies) else 'reorder_acyclic')
    # With the original destination order retained, only sources overwritten by
    # an earlier destination need an early snapshot. Keep every assignment.
    staged = sorted({i for i, j in dependencies if j < i})
    return dict(graph=kind, dependencies=dependencies, self_copies=self_copies,
                partial_overlaps=partial, repeated_sources=repeated,
                schedule=order, blocked_moves=sorted(pending),
                in_order_staged_moves=staged,
                in_order_direct_moves=[i for i in range(len(moves)) if i not in staged],
                preserves_final_a_nz=not pending and (not order or order[-1] == len(moves)-1))


def copy_instructions(edge):
    """Independent exact encoding expectation; rejects unhandled source forms."""
    moves, form = edge['moves'], edge['form']
    if form == 'empty':
        assert not moves
        return []
    word = form in ('direct_word', 'staged_word')
    assert form in ('direct_word', 'staged_word', 'staged_byte')
    assert (form != 'direct_word' or len(moves) == 1)

    def load_source(source, byte=0):
        if source['kind'] == 'immediate':
            value = source['value'] if word else (source['value'] >> (8*byte)) & 255
            return b'\xa9' + value.to_bytes(2 if word else 1, 'little')
        assert source['kind'] in ('stack', 'dp'), ('unsupported copy source', source)
        assert not word or source['kind'] == 'stack'
        return bytes([0xa3 if source['kind'] == 'stack' else 0xa5, source['offset']+byte])

    def store(destination, byte=0):
        assert destination['kind'] in ('stack', 'dp')
        return bytes([0x83 if destination['kind'] == 'stack' else 0x85, destination['offset']+byte])

    for i, m in enumerate(moves):
        assert m['width'] == m['source']['width'] == m['destination']['width']
        assert m['staging']['width'] == 4 and m['staging']['kind'] == 'stack'
        assert not any(overlap(m['destination'], n['destination']) for n in moves[:i])
        assert not any(overlap(m['staging'], n[k]) for n in moves for k in ('source', 'destination'))
        assert not any(overlap(m['staging'], n['staging']) for n in moves[:i])
        if word:
            assert m['width'] == 2 and m['source']['word_operand'] and m['destination']['kind'] == 'stack'
    if form == 'direct_word':
        m = moves[0]
        return [load_source(m['source']), store(m['destination'])]
    ins = []
    for m in moves:
        for byte in ([0] if word else range(m['width'])):
            ins.extend([load_source(m['source'], byte), store(m['staging'], byte)])
    for m in moves:
        for byte in ([0] if word else range(m['width'])):
            ins.extend([bytes([0xa3, m['staging']['offset']+byte]), store(m['destination'], byte)])
    if not word:
        ins.append(b'\xc2\x20')
    return ins


def validate_edge(edge, instructions):
    pc = edge['transfer_pc']
    if edge['fallthrough']:
        assert pc == edge['target_pc']
    else:
        assert instructions[pc] == b'\x5c' + edge['target_pc'].to_bytes(3, 'little')
    expected = copy_instructions(edge)
    start = pc - sum(map(len, expected))
    assert start >= edge['block_pc']
    sites = []
    for code in expected:
        assert instructions.get(start) == code, ('copy bytes', hex(start), instructions.get(start), code)
        sites.append(start)
        start += len(code)
    return sites


def inventory(directory, facts_path, qualification):
    manifest, records = load(directory)
    facts = json.loads(facts_path.read_text())
    assert facts['schema'] == 1
    assert facts['lf_crlf_images_equal'] is True
    q = json.loads(qualification.read_text())
    assert q['slice'] == '3c'
    hashes = {}
    def check_hash(path, expected):
        actual = digest(path)
        assert actual == expected, ('changed evidence', str(path))
        hashes[str(path)] = actual
    for name, expected in q['comparison_hashes'].items():
        check_hash(directory/name, expected)
    # Bind reuse of old execution counts to the exact measured compiler and fixtures.
    for name, expected in q['compiler_and_fixture_inputs'].items():
        check_hash(ROOT/name, expected)
    for name, expected in manifest['inputs'].items():
        check_hash(ROOT/name, expected)
    for a in manifest['artifacts']:
        for name, expected in a['hashes'].items():
            check_hash(Path(a['directory'])/name, expected)
    artifacts = {(a['case'], a['mode']): a for a in manifest['artifacts'] if a['compiler'] == 'actionc'}
    assert len(artifacts) == len(facts['builds']) == 28
    assert set(artifacts) == {(b['case'], b['mode']) for b in facts['builds']}
    summary = Counter()
    forecasts, selective_forecasts, groups = [], [], []
    for build in facts['builds']:
        key = build['case'], build['mode']
        artifact = artifacts[key]
        instructions = dict(listing(Path(artifact['directory'])/'code.asm'))
        measured = [r for k, r in records.items() if k[:3] == (*key, 'actionc')]
        assert measured and all(r['correct'] for r in measured)
        expected_word = {r['vector']: {} for r in measured}
        expected_direct = {r['vector']: {} for r in measured}
        used_sites = set()
        static = Counter()
        for routine in build['routines']:
            placed = next(r for r in artifact['routines'] if r['id'] == routine['id'])
            assert (placed['address'], placed['size'], placed['fixed_frame']) == (routine['address'], routine['size'], routine['frame_extent'])
            used_staging = set()
            for e in routine['edges']:
                assert routine['address'] <= e['block_pc'] <= e['transfer_pc'] < routine['address']+routine['size']
                assert routine['address'] <= e['target_pc'] < routine['address']+routine['size']
                pcs = validate_edge(e, instructions)
                assert not used_sites.intersection(pcs), 'overlapping copies'
                used_sites.update(pcs)
                e['copy_pcs'] = pcs
                e['analysis'] = classify(e['moves'])
                e['executions'] = []
                static[e['form']+'_edges'] += 1
                static['assignments'] += len(e['moves'])
                static['self_copies'] += len(e['analysis']['self_copies'])
                if e['moves']:
                    static[e['analysis']['graph']+'_edges'] += 1
                # Bounded first-slice forecast: retain move order and every copy,
                # including self-copies. Cycles/reordering/byte paths stay staged.
                candidate = (e['form'] == 'staged_word' and not e['analysis']['partial_overlaps']
                             and e['analysis']['graph'] in ('independent','ordered_acyclic'))
                e['in_order_word_candidate'] = candidate
                if candidate:
                    static['candidate_edges'] += 1
                    static['candidate_words'] += len(e['moves'])
                selective = (e['analysis']['in_order_direct_moves'] if e['form'] == 'staged_word'
                             and not e['analysis']['partial_overlaps'] else [])
                e['selective_staging_candidate_moves'] = selective
                if selective:
                    static['selective_candidate_edges'] += 1
                    static['selective_candidate_words'] += len(selective)
                if e['form'].startswith('staged'):
                    for m in e['moves']:
                        used_staging.update(range(m['staging']['offset'], m['staging']['offset']+m['width']))
                for r in measured:
                    counts = r['instruction_sites']
                    # Empty fallthrough can share a PC with another logical edge;
                    # do not invent an execution count for a zero-byte operation.
                    n = counts.get(str(pcs[0]), 0) if pcs else None
                    if pcs:
                        assert all(counts.get(str(pc),0) == n for pc in pcs), ('partial edge',key,e,r['vector'])
                        if not e['fallthrough']:
                            assert counts.get(str(e['transfer_pc']),0) == n
                        e['executions'].append(dict(vector=r['vector'], count=n))
                        summary[e['form']+'_executions'] += n
                        summary['assignment_executions'] += n*len(e['moves'])
                        summary['self_copy_executions'] += n*len(e['analysis']['self_copies'])
                        summary[e['analysis']['graph']+'_executions'] += n
                        if e['form'] in ('direct_word','staged_word') and n:
                            expected_word[r['vector']][str(pcs[0])] = n
                            if e['form'] == 'direct_word': expected_direct[r['vector']][str(pcs[0])] = n
                    if candidate:
                        forecasts.append(dict(case=key[0],mode=key[1],routine=routine['id'],block=e['block'],arm=e['arm'],
                            vector=r['vector'],executions=n,words=len(e['moves']),
                            bytes=4*len(e['moves']),instructions=2*n*len(e['moves']),cycles=10*n*len(e['moves']),
                            stack_reads=2*n*len(e['moves']),stack_writes=2*n*len(e['moves'])))
                    if selective:
                        selective_forecasts.append(dict(case=key[0],mode=key[1],routine=routine['id'],block=e['block'],arm=e['arm'],
                            vector=r['vector'],executions=n,direct_moves=selective,words=len(selective),
                            bytes=4*len(selective),instructions=2*n*len(selective),cycles=10*n*len(selective),
                            stack_reads=2*n*len(selective),stack_writes=2*n*len(selective)))
            reserved = {i for s in routine['staging_slots'] for i in range(s['offset'], s['offset']+s['width'])}
            assert used_staging <= reserved
            routine['staging_bytes_written_statically'] = sorted(used_staging)
            routine['staging_bytes_never_written'] = sorted(reserved-used_staging)
            # This is an allocation inventory, not a proposed new frame extent.
            static['reserved_staging_bytes'] += len(reserved)
            static['staging_bytes_never_written'] += len(reserved-used_staging)
        for r in measured:
            assert r['word_edge_sites'] == expected_word[r['vector']], ('word coverage',key,r['vector'])
            assert r['direct_word_edge_sites'] == expected_direct[r['vector']], ('direct coverage',key,r['vector'])
            assert r['edge_words'] == sum(len(e['moves'])*next(x['count'] for x in e['executions'] if x['vector']==r['vector'])
                for rt in build['routines'] for e in rt['edges'] if e['form'] in ('direct_word','staged_word'))
        summary.update(static)
        groups.append(dict(case=key[0],mode=key[1],**dict(sorted(static.items()))))
    for k in ('empty_edges','direct_word_edges','staged_word_edges','staged_byte_edges',
              'self_copies','self_copy_executions','reorder_acyclic_edges','staged_byte_executions'):
        summary.setdefault(k,0)
    return dict(schema=1,baseline='qualified control-flow slice 3c',
                lf_crlf_images_equal=True,
                counts='Per incoming I state; saved debug/release results are identical. Empty edges have no copy count.',
                scope=dict(builds=28,records=len(records),action_records=sum(k[2]=='actionc' for k in records)),
                evidence_hashes=hashes,facts_sha256=digest(facts_path),qualification_sha256=digest(qualification),
                tool_hashes={str(p.relative_to(ROOT)):digest(p) for p in [Path(__file__),ROOT/'tests/mir65816_copy_inventory.rs']},
                summary=dict(sorted(summary.items())),groups=groups,builds=facts['builds'],
                in_order_word_forecasts=forecasts,
                selective_staging_forecasts=selective_forecasts,
                external_failures=[list(k) for k,r in records.items() if not r['correct']])


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('baseline',type=Path)
    p.add_argument('--facts',type=Path,required=True)
    p.add_argument('--qualification',type=Path,default=ROOT/'docs/abi/action65816-control-flow-3c-qualification.json')
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--check',action='store_true',help='Verify an existing inventory without rewriting it')
    a = p.parse_args()
    result = inventory(a.baseline,a.facts,a.qualification)
    if a.check:
        # Dependency pairs are tuples internally and arrays in the JSON record.
        assert json.loads(a.output.read_text()) == json.loads(json.dumps(result)), 'inventory is stale'
    else:
        a.output.parent.mkdir(parents=True,exist_ok=True)
        a.output.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result['summary'],indent=2))


if __name__ == '__main__':
    main()
