"""Instruction proof must reject every change beyond the selected staging pair."""
import unittest
from check_single_word_edges import check_listing


class DirectCopyListing(unittest.TestCase):
    def setUp(self):
        self.before = [(0x10000, bytes.fromhex('A304')), (0x10002, bytes.fromhex('8310')),
                       (0x10004, bytes.fromhex('A310')), (0x10006, bytes.fromhex('8302')),
                       (0x10008, bytes.fromhex('5C0C0001')), (0x1000c, b'\x6b')]
        self.after = [(0x10000, bytes.fromhex('A304')), (0x10002, bytes.fromhex('8302')),
                      (0x10004, bytes.fromhex('5C080001')), (0x10008, b'\x6b')]

    def test_exact_removal_and_relocation(self):
        self.assertEqual(check_listing(self.before, self.after, {0x10000}), {0x10000})
        self.assertEqual(check_listing(self.before, self.before, set()), set())
        with self.assertRaises(AssertionError): check_listing(self.before, self.after, set())

    def test_operands_targets_and_missing_instructions_are_not_exempt(self):
        for i, code in [(0, bytes.fromhex('A305')), (1, bytes.fromhex('8303')),
                        (2, bytes.fromhex('5C090001')), (3, b'\xea')]:
            after = self.after.copy()
            after[i] = (after[i][0], code)
            with self.subTest(i=i), self.assertRaises(AssertionError):
                check_listing(self.before, after, {0x10000})
        with self.assertRaises(AssertionError): check_listing(self.before, self.after[:-1], {0x10000})
        with self.assertRaises(AssertionError): check_listing(self.before, self.after, {0x10004})
