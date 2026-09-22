import copy
import itertools
from pathlib import Path
import tempfile
import unittest

from inventory_copies import classify, copy_instructions, validate_edge
from check_empty_edges import listing


def move(src, dst, stage=32, width=2):
    return dict(source=dict(kind='stack', offset=src, width=width, word_operand=width == 2),
                destination=dict(kind='stack', offset=dst, width=width),
                staging=dict(kind='stack', offset=stage, width=4), width=width)


class CopyInventory(unittest.TestCase):
    def test_chains_need_directional_dependencies(self):
        forward = [move(2, 4), move(6, 2, 36)]
        backward = list(reversed(forward))
        self.assertEqual(classify(forward)['graph'], 'ordered_acyclic')
        self.assertEqual(classify(forward)['in_order_staged_moves'], [])
        self.assertEqual(classify(backward)['graph'], 'reorder_acyclic')
        self.assertEqual(classify(backward)['in_order_staged_moves'], [1])
        self.assertFalse(classify(backward)['preserves_final_a_nz'])

    def test_cycle_plus_independent_counter(self):
        a = classify([move(6, 10), move(10, 6, 36), move(12, 8, 40)])
        self.assertEqual(a['graph'], 'cyclic')
        self.assertEqual(a['blocked_moves'], [0, 1])
        self.assertEqual(a['schedule'], [2])
        self.assertEqual(a['in_order_staged_moves'], [1])
        self.assertEqual(a['in_order_direct_moves'], [0, 2])

    def test_self_copy_repeated_source_and_partial_overlap_are_visible(self):
        a = classify([move(2, 2), move(2, 6, 36), move(3, 8, 40)])
        self.assertEqual(a['self_copies'], [0])
        self.assertEqual(a['repeated_sources'], [(1, 0)])
        self.assertIn((0, 2), a['partial_overlaps'])
        self.assertEqual(classify([])['graph'], 'empty')

    def test_selective_snapshot_model_matches_simultaneous_assignment(self):
        # Exhaustive small graphs, including swaps, cycles, repeated sources,
        # self-copies, and byte-overlapping sources. Oracle captures all inputs.
        for sources in itertools.product(range(2, 8), repeat=3):
            moves = [move(src, dst, 32+4*i) for i, (src, dst) in enumerate(zip(sources, (2, 4, 6)))]
            original = bytearray(range(64))
            values = [original[s:s+2] for s in sources]
            expected = original.copy()
            for m, value in zip(moves, values):
                d = m['destination']['offset']; expected[d:d+2] = value
            facts = classify(moves)
            saved = {i: original[sources[i]:sources[i]+2] for i in facts['in_order_staged_moves']}
            actual = original.copy()
            last = None
            for i, m in enumerate(moves):
                s, d = sources[i], m['destination']['offset']
                last = saved[i] if i in saved else actual[s:s+2]
                actual[d:d+2] = last
            self.assertEqual(actual, expected)
            self.assertEqual(last, values[-1])  # A and therefore word N/Z.

    def test_exact_copy_bytes_and_corruption_rejection(self):
        e = dict(moves=[move(2, 6), move(4, 8, 36)], form='staged_word',
                 block_pc=0, transfer_pc=16, target_pc=30, fallthrough=False)
        expected = [bytes.fromhex(s) for s in ['a302','8320','a304','8324','a320','8306','a324','8308']]
        self.assertEqual(copy_instructions(e), expected)
        code = {2*i: b for i, b in enumerate(expected)}
        code[16] = bytes.fromhex('5c1e0000')
        self.assertEqual(validate_edge(e, code), list(range(0,16,2)))
        for pc in (0, 6, 12, 16):
            bad = dict(code); bad[pc] = b'\xea'
            with self.assertRaises(AssertionError): validate_edge(e, bad)
        bad = copy.deepcopy(e); bad['moves'][1]['staging']['offset'] = 32
        with self.assertRaises(AssertionError): copy_instructions(bad)

    def test_byte_fallback_and_zero_byte_transfer(self):
        e = dict(moves=[move(2, 6, width=1)], form='staged_byte',
                 block_pc=0, transfer_pc=10, target_pc=10, fallthrough=True)
        expected = [bytes.fromhex(s) for s in ['a302','8320','a320','8306','c220']]
        self.assertEqual(copy_instructions(e), expected)
        code = {2*i: b for i, b in enumerate(expected)}
        self.assertEqual(validate_edge(e, code), [0,2,4,6,8])
        e.update(moves=[],form='empty')
        self.assertEqual(validate_edge(e, code), [])

    def test_listing_crlf_through_the_actual_inventory_decoder(self):
        text = '000000  A3 02       LDA $02,S\n000002  83 06       STA $06,S\n'
        e = dict(moves=[move(2,6)],form='direct_word',block_pc=0,
                 transfer_pc=4,target_pc=4,fallthrough=True)
        with tempfile.TemporaryDirectory() as directory:
            p = Path(directory)/'code.asm'
            for source in (text, text.replace('\n','\r\n')):
                p.write_bytes(source.encode())
                self.assertEqual(validate_edge(e,dict(listing(p))),[0,2])


if __name__ == '__main__':
    unittest.main()
