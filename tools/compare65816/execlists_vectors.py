"""Independent list-state oracle for the frozen Exec816 list comparison."""
from copy import deepcopy

SIGNATURES = {
    'NewList': ([3], 0), 'NewMinList': ([3], 0),
    'IsListEmpty': ([3], 1), 'IsMinListEmpty': ([3], 1),
    'AddHead': ([3, 3], 0), 'AddTail': ([3, 3], 0),
    'Insert': ([3, 3, 3], 0), 'Remove': ([3], 0),
    'RemHead': ([3], 3), 'RemTail': ([3], 3),
    'Enqueue': ([3, 3], 0), 'FindName': ([3, 3], 3),
}


def cases():
    result = []
    for name, (args, returns) in SIGNATURES.items():
        vectors = []
        scenarios = range(6) if name in ('Insert', 'Enqueue', 'FindName') else range(3)
        for placement in ('far', 'bank_crossing', 'bank_zero'):
            h = 0x12fffe if placement == 'bank_crossing' else 0x120101
            nodes = [0x230211, 0x340327, 0x45043d]
            item = 0x560553
            names = [0x67fffe, 0x780701, 0x890801]
            query = 0x9afffe
            if placement == 'bank_zero':
                h, nodes, item = 0x8100, [0x8200, 0x8300, 0x8400], 0x8500
                names, query = [0x9000, 0x9100, 0x9200], 0x9300
            for scenario in scenarios:
                count = min(scenario, 2)
                if name == 'Remove':
                    count = 3
                if name == 'Insert':
                    count = 2
                if name in ('Enqueue', 'FindName'):
                    count = 3
                order = nodes[:count]
                # Use complete full records; metadata distinguishes unwanted
                # fourth-byte pointer stores from legitimate link writes.
                mem = {h: bytearray([0xd6] * 11), item: bytearray([0xe7] * 11)}
                mem.update({n: bytearray([0xa1+i] * 11) for i, n in enumerate(nodes)})
                mem.update({a: bytearray(s) for a, s in zip(names, [b'Alpha\0', b'Alpha\0', b'Beta\0'])})
                mem[query] = bytearray(b'Alpha\0')

                def put(at, value):
                    for base, data in mem.items():
                        if base <= at and at+3 <= base+len(data):
                            data[at-base:at-base+3] = value.to_bytes(3, 'little')
                            return
                    raise AssertionError(hex(at))

                def link(sequence):
                    put(h, sequence[0] if sequence else h+3)
                    put(h+3, 0)
                    put(h+6, sequence[-1] if sequence else h)
                    for i, n in enumerate(sequence):
                        put(n, sequence[i+1] if i+1 < len(sequence) else h+3)
                        put(n+3, sequence[i-1] if i else h)

                link(order)
                for n, a, priority in zip(nodes, names, [127, 0, 128]):
                    put(n+8, a)
                    mem[n][7] = priority
                argv = [h]
                expected = None
                if name == 'FindName':
                    if scenario == 0: order = []; link(order)
                    if scenario == 2: put(nodes[0]+8, 0)
                    if scenario == 3: mem[query] = bytearray(b'Beta\0')
                    if scenario == 4: mem[query] = bytearray(b'Missing\0')
                    if scenario == 5: argv = [nodes[0]]; order = order[1:]
                    argv.append(query)
                    expected = 0
                    for n in order:
                        ptr = int.from_bytes(mem[n][8:11], 'little')
                        if ptr and mem[ptr] == mem[query]: expected = n; break
                if name == 'Enqueue':
                    priority = [127, 126, 0, 255, 128, 129][scenario]
                    mem[item][7] = priority
                    argv.append(item)
                if name in ('NewList', 'NewMinList'):
                    mem[h][:9] = bytes([0x9c] * 9)
                before = deepcopy(mem)
                if name in ('NewList', 'NewMinList'):
                    link([])
                elif name in ('IsListEmpty', 'IsMinListEmpty'):
                    expected = int(not order)
                elif name == 'AddHead':
                    argv.append(item); link([item]+order)
                elif name == 'AddTail':
                    argv.append(item); link(order+[item])
                elif name == 'Insert':
                    pred = [0, h, h+3, nodes[0], nodes[1], 0][scenario]
                    if scenario == 5: order = []; link(order); before = deepcopy(mem)
                    argv.extend([item, pred])
                    index = 0 if pred in (0, h) else len(order) if pred == h+3 else order.index(pred)+1
                    link(order[:index]+[item]+order[index:])
                elif name == 'Remove':
                    argv = [nodes[scenario]]; link([n for n in order if n != argv[0]])
                elif name in ('RemHead', 'RemTail'):
                    expected = (order[0] if name == 'RemHead' else order[-1]) if order else 0
                    if expected: link([n for n in order if n != expected])
                elif name == 'Enqueue':
                    signed = lambda v: v if v < 128 else v-256
                    index = next((i for i,n in enumerate(order) if signed(priority) > signed(mem[n][7])), len(order))
                    link(order[:index]+[item]+order[index:])
                # Relinking updates only surviving nodes; removed links remain
                # stale as the library contract requires.
                encode = lambda m: [dict(address=a, bytes=list(v)) for a,v in sorted(m.items())]
                vectors.append(dict(args=argv, result=expected, memory=encode(before), after=encode(mem), placement=placement, scenario=scenario))
        result.append(dict(id=name, args=args, returns=returns, vectors=vectors))
    return result
