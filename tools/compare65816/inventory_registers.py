#!/usr/bin/env python3
"""Inventory current emitted memory traffic and X/Y use without predicting savings."""
import argparse
import csv
import io
from collections import Counter
import json
from pathlib import Path
import sys
sys.dont_write_bytecode = True
from check_selective_staging import all_instructions
from delta import load
from inventory_movements import interference
from report import PICKS, digest

ROOT = Path(__file__).resolve().parents[2]
# These opcodes are decoded independently from final bytes, never from MIR effects.
DP_READ = {0xa6, 0xa5, 0x65, 0xe5, 0xc5, 0x25, 0x05, 0x45}
DP_RMW = {0x06, 0x26, 0x46, 0x66}
STACK_READ = {0xa3, 0x63, 0xe3, 0xc3}
INDIRECT = {0xa7, 0x87, 0xb7, 0x97}
# Fail closed if the shared disassembler later learns another instruction.
SUPPORTED = DP_READ | DP_RMW | STACK_READ | INDIRECT | {
    0x85, 0x83, 0xaf, 0x8f, 0x22, 0x5c, 0x6b, 0x48, 0x4b, 0x62,
    0xaa, 0xa2, 0xca, 0x8a, 0xa8, 0xa0, 0x98, 0xc2, 0xe2,
    0x18, 0x38, 0x1b, 0x3b, 0xeb, 0x3a, 0xa9, 0x69, 0xe9,
    0xc9, 0xe0, 0x29, 0x49, 0x10, 0x30, 0x90, 0xb0, 0xd0, 0xf0,
}


def scope(r, pc):
    matches = [(b['id'], o) for b in r['blocks'] for o in b['ops']
               if o['range'][0] <= pc < o['range'][1]]
    assert len(matches) <= 1, ('overlapping MIR spans', pc)
    if not matches:
        assert pc < min(b['pc'] for b in r['blocks']), ('unowned body instruction', pc)
        return dict(kind='entry'), None
    block, op = matches[0]
    return dict(kind=op['kind'], block=block, operation=op['index']), op


def covers(start, size, offset, width):
    return start <= offset and offset + width <= start + size


def owner(r, space, offset, width, call=False):
    if space == 'stack' and call:
        # Call spans adjust S; body displacements cannot identify these accesses.
        return dict(kind='call_span_stack')
    temps = [t['id'] for t in r['temp_homes'] if t['home']['kind'] == space
             and covers(t['home']['offset'], t['home']['width'], offset, width)]
    if temps:
        return dict(kind='private_temp', temps=sorted(temps))
    if space == 'dp':
        assert offset + width <= 256
        return dict(kind='metadata' if offset >= 64 else 'selector_scratch')
    found = []
    for o in r['fixed_objects']:
        if covers(o['offset'], o['width'], offset, width):
            mutable_param = any(p['object'] == o['id'] for p in r['parameters'])
            found.append(dict(kind='mutable_parameter' if mutable_param else 'fixed_object',
                              object=o['id'], addressable=o['addressable']))
    for i, slot in enumerate(r['staging_slots']):
        if covers(slot['offset'], slot['width'], offset, width):
            found.append(dict(kind='edge_staging', slot=i))
    for a in r['arguments']:
        if covers(a['body_displacement'], a['size'], offset, width):
            found.append(dict(kind='incoming_argument', argument_offset=a['offset']))
    assert len(found) == 1, ('unclassified/ambiguous body stack access', offset, width, found)
    return found[0]


def instructions(r, decoded):
    """Linear M/X tracking follows the native image/disassembler contract."""
    result = []
    m8 = x8 = False
    for pc, code in decoded:
        if not r['address'] <= pc < r['address'] + r['size']:
            continue
        op = code[0]
        assert op in SUPPORTED, ('unmodelled instruction', pc, code.hex())
        context, mir = scope(r, pc)
        row = dict(pc=pc, bytes=code.hex(), scope=context, a_bytes=1 if m8 else 2,
                   index_bytes=1 if x8 else 2, reads=[], writes=[], memory=[])
        width = row['a_bytes']
        def memory(space, offset, size, read, write):
            event = dict(space=space, offset=offset, width=size, read=read, write=write)
            event['owner'] = owner(r, space, offset, size, mir and mir['kind'] == 'call')
            row['memory'].append(event)
        if op in DP_READ | DP_RMW | {0x85}:
            memory('dp', code[1], row['index_bytes'] if op == 0xa6 else width,
                   int(op != 0x85), int(op in DP_RMW or op == 0x85))
        elif op in STACK_READ | {0x83}:
            memory('stack', code[1], width, int(op != 0x83), int(op == 0x83))
        elif op in INDIRECT:
            memory('dp', code[1], 3, 1, 0)
            row['external_access'] = dict(width=width, read=int(op in (0xa7, 0xb7)),
                                          write=int(op in (0x87, 0x97)), indirect=True)
        elif op in (0xaf, 0x8f):
            row['external_access'] = dict(width=width, read=int(op == 0xaf),
                                          write=int(op == 0x8f), indirect=False,
                                          address=int.from_bytes(code[1:], 'little'))
        if op in (0x22, 0x6b, 0x48, 0x4b, 0x62):
            size = {0x22: 3, 0x6b: 3, 0x48: width, 0x4b: 1, 0x62: 2}[op]
            row['memory'].append(dict(space='stack', offset=None, width=size,
                read=int(op == 0x6b), write=int(op != 0x6b),
                owner=dict(kind='return_address' if op in (0x22, 0x6b, 0x4b, 0x62) else 'push')))
        if op in (0xaa, 0xa6, 0xa2, 0xca): row['writes'].append('x')
        if op in (0x8a, 0xca, 0xe0): row['reads'].append('x')
        if op in (0xa8, 0xa0): row['writes'].append('y')
        if op in (0x98, 0xb7, 0x97): row['reads'].append('y')
        if op in (0xc2, 0xe2):
            if code[1] & 0x20: m8 = op == 0xe2
            if code[1] & 0x10:
                x8 = op == 0xe2
                if x8: row['writes'] += ['x_high', 'y_high']
        # ABI clobbers are obligations, distinct from explicit register writes.
        if op == 0x22: row['call_clobbers'] = ['a', 'x', 'y', 'flags', 'dp_scratch']
        # Only plain word LDA costs are budgeted: aligned D, native mode, fixed
        # addresses. This is existing cost, not an additive removal forecast.
        if op in (0xa3, 0xa5) and width == 2:
            row['word_load_cycles'] = 5 if op == 0xa3 else 4
        result.append(row)
    assert sum(len(bytes.fromhex(i['bytes'])) for i in result) == r['size']
    return result


def loops(r):
    """Natural backedges from dominance, not textual block/PC ordering."""
    blocks = {b['id']: b for b in r['blocks']}
    entry = r['blocks'][0]['id']
    reachable = {entry}
    while True:
        after = reachable | {s for b in reachable for s in blocks[b]['successors']}
        if after == reachable: break
        reachable = after
    pred = {b: {p for p in reachable if b in blocks[p]['successors']} for b in reachable}
    dom = {b: ({b} if b == entry else set(reachable)) for b in reachable}
    while True:
        old = {b: set(d) for b, d in dom.items()}
        for b in reachable - {entry}:
            dom[b] = {b} | set.intersection(*(dom[p] for p in pred[b]))
        if old == dom: break
    points = interference(r)
    homes = {t['id']: t for t in r['temp_homes']}
    definitions = {o['definition']: (b['id'], o) for b in r['blocks'] for o in b['ops']
                   if o['definition'] is not None}
    result = []
    for e in r['edges']:
        latch, header = e['block'], e['target_block']
        if latch not in reachable or header not in dom[latch]: continue
        members = {header, latch}
        pending = [latch] if latch != header else []
        while pending:
            for p in pred[pending.pop()] - members:
                members.add(p)
                if p != header: pending.append(p)
        params = []
        for move in e['moves']:
            dest, source = move['destination_temp'], move['source'].get('temp')
            definition = definitions.get(source)
            update = None
            if definition:
                block, op = definition
                update = dict(block=block, operation=op['index'], kind=op['kind'],
                              binary=op.get('binary'), values=op.get('values', []))
            witnesses = [p for p, live in points if source != dest and source in live and dest in live]
            params.append(dict(temp=dest, home=homes[dest]['home'], type=homes[dest]['type'],
                               backedge_source=move['source'], update=update,
                               closed_operation_conflicts=witnesses))
        result.append(dict(header=header, latch=latch, blocks=sorted(members), parameters=params))
    return result


def measure(row, sites):
    counts = {int(pc): n for pc, n in row['instruction_sites'].items()}
    assert set(counts) <= sites.keys()
    assert sum(counts.values()) == row['instructions']
    traffic, registers, loads, home_traffic = Counter(), Counter(), Counter(), Counter()
    for pc, n in counts.items():
        i = sites[pc]
        for access in ('reads', 'writes'):
            for reg in i[access]: registers[f"{i['scope']['kind']}:{reg}:{access}"] += n
        for e in i['memory']:
            space, kind = e['space'], e['owner']['kind']
            for access in ('read', 'write'):
                value = n * e['width'] * e[access]
                if value:
                    traffic[f'{space}:{kind}:{access}'] += value
                    if e['offset'] is not None:
                        home_traffic[f"r{i['routine']}:{space}:{e['offset']:02x}:{kind}:{access}"] += value
        if 'word_load_cycles' in i:
            space = i['memory'][0]['space']
            loads[f'{space}:executions'] += n
            loads[f'{space}:existing_cycles'] += n * i['word_load_cycles']
    for space in ('stack', 'dp'):
        for access in ('read', 'write'):
            actual = sum(n for k, n in traffic.items() if k.startswith(space + ':')
                         and ':metadata:' not in k and k.endswith(':' + access))
            assert actual == row[f'{space}_{access}s'], (row['case'], row['mode'], space, access, actual, row[f'{space}_{access}s'])
    assert traffic['dp:metadata:read'] == row['metadata_reads']
    return dict(vector=row['vector'], args=row['args'], cycles=row['cycles'],
                instructions=row['instructions'], instruction_sites=row['instruction_sites'], code_bytes=row['code_bytes'],
                peak_below_entry_s=row['peak_below_entry_s'], traffic=dict(sorted(traffic.items())),
                registers=dict(sorted(registers.items())), word_loads=dict(sorted(loads.items())),
                physical_home_traffic=dict(sorted(home_traffic.items())))


def inventory(directory, facts):
    manifest, records = load(directory)
    assert facts['schema'] == 2 and facts['lf_crlf_images_equal']
    expected = {(c['id'], mode, compiler, v) for c in manifest['cases']
                for v in range(len(c['vectors'])) for mode in ('raw', 'optimized')
                for compiler in ('actionc', 'vbcc')}
    assert set(records) == expected
    for p, sha in manifest['inputs'].items(): assert digest(ROOT/p) == sha, p
    for a in manifest['artifacts']:
        for p, sha in a['hashes'].items(): assert digest(Path(a['directory'])/p) == sha, p
    build_keys = {(b['case'], b['mode']) for b in facts['builds']}
    assert len(build_keys) == len(facts['builds']) == 28
    assert build_keys == {(a['case'], a['mode']) for a in manifest['artifacts'] if a['compiler'] == 'actionc'}
    builds = []
    for f in facts['builds']:
        case, mode = f['case'], f['mode']
        a, = [a for a in manifest['artifacts'] if (a['case'], a['mode'], a['compiler']) == (case, mode, 'actionc')]
        image = json.loads(Path(a['image']).read_text())
        decoded = all_instructions(image)
        assert len(f['routines']) == len(image['routines'])
        routines, sites = [], {}
        for r in f['routines']:
            placed, = [p for p in image['routines'] if p['id'] == r['id']]
            assert all(r[k] == placed[k] for k in ('address', 'size', 'arguments', 'objects', 'calls'))
            assert r['counted'] == any(start == r['address'] and end == r['address'] + r['size']
                                       for start, end in a['code_ranges'])
            code = instructions(r, decoded)
            for i in code:
                i['routine'] = r['id']
                assert i['pc'] not in sites
                sites[i['pc']] = i
            barriers = [dict(block=b['id'], operation=o['index'], kind=o['kind'], range=o['range'])
                        for b in r['blocks'] for o in b['ops'] if o.get('effects', {}).get('barrier')]
            logical = [m for e in r['edges'] for m in e['moves']]
            routines.append(dict(id=r['id'], counted=r['counted'], frame=r['frame_extent'],
                address=r['address'], size=r['size'], loops=loops(r), barriers=barriers,
                logical_edge_assignments=len(logical), same_home_assignments=sum(
                    m['source']['kind'] == m['destination']['kind'] and
                    m['source'].get('offset') == m['destination']['offset'] and
                    m['source']['width'] == m['destination']['width'] for m in logical),
                instructions=code))
        assert sites.keys() == {pc for pc, _ in decoded}, 'uncovered executable bytes'
        measured = []
        for key, row in sorted(records.items()):
            if key[:3] != (case, mode, 'actionc'): continue
            assert all(any(start <= int(pc) < end for start, end in a['code_ranges'])
                       for pc in row['instruction_sites'])
            measured.append(measure(row, sites))
        assert all(r['correct'] for k, r in records.items() if k[:3] == (case, mode, 'actionc'))
        builds.append(dict(case=case, mode=mode, image_sha256=digest(a['image']), routines=routines, measurements=measured))
    assert len(builds) == 28
    counted = [r for b in builds for r in b['routines'] if r['counted']]
    static = [i for r in counted for i in r['instructions']]
    summary = dict(builds=len(builds), routines=sum(len(b['routines']) for b in builds),
                   counted_routines=len(counted), records=sum(len(b['measurements']) for b in builds),
                   counted_instruction_sites=len(static),
                   word_load_sites=dict(Counter(i['memory'][0]['space'] for i in static if 'word_load_cycles' in i)))
    return dict(schema=1, summary=summary, compiler_revision=manifest['compiler_revision'],
        debug_release_identical=True, lf_crlf_images_equal=True,
        inputs_sha256=manifest['inputs'],
        incorrect=[{k: r[k] for k in ('case', 'mode', 'compiler', 'vector', 'errors')}
                   for r in records.values() if not r['correct']], builds=builds)


def table(result):
    lines = ['# Current memory and register inventory', '',
        'VM cycles and byte traffic are per invocation and per incoming I state; both',
        'I states agree, as do both host builds. Loads are executed word LDA counts.',
        'DP excludes task metadata. Stack includes argument and return-address traffic.',
        'X/Y columns count explicit body accesses, excluding entry and return sequences;',
        'call clobbers and conservative operation barriers are recorded separately in JSON.', '',
        '| Kernel | Mode | Arguments | Bytes | Cycles | Peak | Stack R/W | DP R/W | Word LDA stack/DP | Body X R/W | Body Y R/W |',
        '| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |']
    for b in result['builds']:
        r, = [r for r in b['measurements'] if r['vector'] == PICKS[b['case']]]
        def traffic(space, access):
            return sum(v for k, v in r['traffic'].items() if k.startswith(space + ':')
                       and ':metadata:' not in k and k.endswith(':' + access))
        def registers(reg, access):
            return sum(v for k, v in r['registers'].items() if k.endswith(f':{reg}:{access}')
                       and not k.startswith(('entry:', 'return:', 'exit:')))
        cells = [b['case'], b['mode'], ', '.join(map(str, r['args'])), r['code_bytes'], r['cycles'], r['peak_below_entry_s'],
                 f"{traffic('stack', 'read')}/{traffic('stack', 'write')}",
                 f"{traffic('dp', 'read')}/{traffic('dp', 'write')}",
                 f"{r['word_loads'].get('stack:executions', 0)}/{r['word_loads'].get('dp:executions', 0)}",
                 f"{registers('x', 'reads')}/{registers('x', 'writes')}",
                 f"{registers('y', 'reads')}/{registers('y', 'writes')}"]
        lines.append('| ' + ' | '.join(map(str, cells)) + ' |')
    lines += ['', 'Known optimized vbcc unlink vector 0 remains incorrect in the source reports.',
              'The complete Action instruction streams and all 132 Action records are inventoried;',
              'uncounted Main wrappers have static facts only. This is not a register-allocation forecast.', '']
    return '\n'.join(lines)


def verify_snapshot(directory, snapshot):
    provenance = json.loads((snapshot/'provenance.json').read_text())
    assert digest(snapshot/'results.csv') == provenance['results_sha256']
    for path, sha in provenance['inputs_sha256'].items():
        assert digest(ROOT/path) == sha, ('changed measurement input', path)
    debug = json.loads((directory/'debug.json').read_text())
    content = io.StringIO(newline='')
    writer = csv.DictWriter(content, fieldnames=list(debug['measurements'][0]), lineterminator='\n')
    writer.writeheader()
    for row in debug['measurements']:
        writer.writerow({k: json.dumps(v, separators=(',', ':')) if isinstance(v, (list, bool)) else v
                         for k, v in row.items()})
    assert content.getvalue() == (snapshot/'results.csv').read_text(), 'changed measured records'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('--facts', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--check', action='store_true')
    parser.add_argument('--snapshot', type=Path, default=ROOT/'docs/benchmarks/65816-scalar-dp/after')
    args = parser.parse_args()
    verify_snapshot(args.directory, args.snapshot)
    result = inventory(args.directory, json.loads(args.facts.read_text()))
    result['facts_sha256'] = digest(args.facts)
    result['source_results_sha256'] = digest(args.snapshot/'results.csv')
    artifacts = {'inventory.json': json.dumps(result, indent=2) + '\n', 'tables.md': table(result)}
    for name, content in artifacts.items():
        path = args.output/name
        if args.check: assert path.read_text() == content, f'stale inventory: {path}'
        else:
            args.output.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
    print(f'Checked {result["summary"]}: {args.output}')


if __name__ == '__main__':
    main()
