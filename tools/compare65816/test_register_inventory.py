import copy
import unittest
from inventory_registers import all_instructions, instructions, loops, measure, owner


def fixture(code, kind='binary'):
    image = dict(format='actionc-65816-image', version=3, abi='action65816.native.v1',
                 segments=[dict(address=0x10000, bytes=list(code), executable=True)])
    r = dict(address=0x10000, size=len(code), fixed_objects=[], parameters=[],
             staging_slots=[], arguments=[], temp_homes=[], blocks=[dict(id=0, pc=0x10000,
             ops=[dict(index=0, kind=kind, range=[0x10000, 0x10000 + len(code)])])])
    return r, all_instructions(image)


def temp(i, kind, offset, width=2):
    return dict(id=i, home=dict(kind=kind, offset=offset, width=width), type=dict(width=width))


class RegisterInventory(unittest.TestCase):
    def test_accumulator_and_index_widths_are_independent(self):
        r, decoded = fixture(bytes.fromhex('e2 20 a5 20 a6 0a c2 20 a5 20 e2 10 a6 0a c2 10'))
        r['temp_homes'] = [temp(0, 'dp', 32)]
        rows = instructions(r, decoded)
        self.assertEqual([i['memory'][0]['width'] for i in rows if i['memory']], [1, 2, 2, 1])
        self.assertEqual(rows[5]['writes'], ['x_high', 'y_high'])
        self.assertNotIn('word_load_cycles', rows[1])
        self.assertEqual(rows[4]['word_load_cycles'], 4)

    def test_home_spaces_mutable_arguments_staging_and_ambiguity(self):
        r, _ = fixture(b'\x6b')
        r['temp_homes'] = [temp(0, 'stack', 32), temp(1, 'dp', 32), temp(2, 'dp', 32)]
        self.assertEqual(owner(r, 'stack', 32, 2)['temps'], [0])
        self.assertEqual(owner(r, 'dp', 32, 2)['temps'], [1, 2])
        r['fixed_objects'] = [dict(id=1, offset=2, width=2, addressable=False)]
        r['parameters'] = [dict(id=0, object=1)]
        r['staging_slots'] = [dict(offset=6, width=2)]
        r['arguments'] = [dict(body_displacement=12, size=2, offset=0)]
        self.assertEqual(owner(r, 'stack', 2, 2)['kind'], 'mutable_parameter')
        self.assertEqual(owner(r, 'stack', 6, 2)['kind'], 'edge_staging')
        self.assertEqual(owner(r, 'stack', 12, 2)['kind'], 'incoming_argument')
        with self.assertRaises(AssertionError): owner(r, 'stack', 3, 2)
        with self.assertRaises(AssertionError): owner(r, 'stack', 9, 1)

    def test_call_spans_do_not_misidentify_transient_stack_homes(self):
        r, decoded = fixture(bytes.fromhex('a3 02 83 04 22 00 00 02'), 'call')
        r['temp_homes'] = [temp(0, 'stack', 2)]
        rows = instructions(r, decoded)
        self.assertEqual(rows[0]['memory'][0]['owner']['kind'], 'call_span_stack')
        self.assertEqual(rows[1]['memory'][0]['owner']['kind'], 'call_span_stack')
        self.assertEqual(rows[2]['call_clobbers'], ['a', 'x', 'y', 'flags', 'dp_scratch'])
        self.assertEqual(rows[2]['writes'], [])  # ABI obligation is not an explicit TAX/TAY.
        self.assertEqual(rows[2]['memory'][0]['width'], 3)

    def test_indexed_pointer_reads_and_rmw_traffic(self):
        r, decoded = fixture(bytes.fromhex('b7 00 97 03 26 08 ca'))
        rows = instructions(r, decoded)
        self.assertEqual(rows[0]['reads'], ['y'])
        self.assertEqual(rows[0]['memory'][0]['width'], 3)
        self.assertEqual(rows[1]['external_access'], dict(width=2, read=0, write=1, indirect=True))
        self.assertEqual((rows[2]['memory'][0]['read'], rows[2]['memory'][0]['write']), (1, 1))
        self.assertEqual((rows[3]['reads'], rows[3]['writes']), (['x'], ['x']))

    def test_push_and_return_widths(self):
        r, decoded = fixture(bytes.fromhex('48 e2 20 48 4b 62 00 00 6b'))
        rows = instructions(r, decoded)
        self.assertEqual([i['memory'][0]['width'] for i in rows if i['memory']], [2, 1, 1, 2, 3])
        self.assertEqual(rows[-1]['memory'][0]['read'], 1)

    def test_unknown_bytes_unowned_code_and_missing_measurements_fail(self):
        with self.assertRaises(ValueError): fixture(b'\xff')
        unknown, _ = fixture(b'\x6b')
        with self.assertRaises(AssertionError): instructions(unknown, [(0x10000, b'\xff')])
        r, decoded = fixture(bytes.fromhex('a5 20 85 22 6b'))
        r['temp_homes'] = [temp(0, 'dp', 32), temp(1, 'dp', 34)]
        rows = instructions(r, decoded)
        sites = {i['pc']: dict(i, routine=0) for i in rows}
        measured = dict(case='test', mode='raw', vector=0, args=[], cycles=14, instructions=3,
                        code_bytes=5, peak_below_entry_s=0, instruction_sites={str(pc): 1 for pc in sites},
                        stack_reads=3, stack_writes=0, dp_reads=2, dp_writes=2, metadata_reads=0)
        result = measure(measured, sites)
        self.assertEqual(result['word_loads'], {'dp:executions': 1, 'dp:existing_cycles': 4})
        for field in ('stack_reads', 'stack_writes', 'dp_reads', 'dp_writes', 'metadata_reads', 'instructions'):
            with self.assertRaises(AssertionError): measure(dict(measured, **{field: measured[field] + 1}), sites)
        bad = copy.deepcopy(measured); bad['instruction_sites'][str(0x10001)] = 1
        with self.assertRaises(AssertionError): measure(bad, sites)
        r['blocks'][0]['ops'][0]['range'][1] -= 1
        with self.assertRaises(AssertionError): instructions(r, decoded)

    def test_natural_loop_uses_dominance_and_retains_closed_operation_conflict(self):
        def block(i, params, successors, ops=(), uses=()):
            return dict(id=i, params=params, successors=successors, ops=list(ops), terminator_uses=list(uses))
        r = dict(blocks=[block(9, [], [7]), block(7, [0], [2, 1]),
                        block(2, [], [7], [dict(index=0, kind='binary', binary='add', uses=[0], definition=1)], [1]),
                        block(1, [], [])],
                 edges=[dict(block=9, target_block=7, moves=[]),
                        dict(block=2, target_block=7, moves=[dict(destination_temp=0, source=dict(temp=1))])],
                 temp_homes=[temp(0, 'dp', 32), temp(1, 'dp', 34)])
        found, = loops(r)
        self.assertEqual((found['header'], found['latch'], found['blocks']), (7, 2, [2, 7]))
        self.assertEqual(found['parameters'][0]['closed_operation_conflicts'],
                         [dict(block=2, operation=0, point='closed_operation')])
        self.assertEqual(found['parameters'][0]['update']['binary'], 'add')


if __name__ == '__main__':
    unittest.main()
