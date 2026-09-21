"""The comparison-specific read accounting must not weaken other checks."""
import copy
import unittest

from delta import PRESERVED, check_records, stack_read_deltas


class StackReadDeltas(unittest.TestCase):
    def setUp(self):
        self.key = ('maximum', 'raw', 'actionc', 3)
        self.row = dict(case='maximum', mode='raw', compiler='actionc', vector=3, delta=2)
        record = dict.fromkeys(PRESERVED, 0)
        record.update(compiler='actionc', correct=True, code_bytes=186, cycles=159,
                      stack_reads=14, errors=[], result=65535)
        self.old = {self.key: record}
        self.new = copy.deepcopy(self.old)
        self.new[self.key].update(code_bytes=146, cycles=151, stack_reads=16)

    def test_default_is_strict_and_explicit_delta_is_exact(self):
        check_records(self.old, self.old, {})
        with self.assertRaises(AssertionError):
            check_records(self.old, self.new, {})
        expected = stack_read_deltas([self.row])
        check_records(self.old, self.new, expected)
        for count in [13, 14, 15, 17]:
            changed = copy.deepcopy(self.new)
            changed[self.key]['stack_reads'] = count
            with self.subTest(count=count), self.assertRaises(AssertionError):
                check_records(self.old, changed, expected)

    def test_other_preserved_fields_fail_even_with_a_read_delta(self):
        for field in PRESERVED:
            changed = copy.deepcopy(self.new)
            changed[self.key][field] = 'changed'
            with self.subTest(field=field), self.assertRaises(AssertionError):
                check_records(self.old, changed, stack_read_deltas([self.row]))
        for field in ['code_bytes', 'cycles']:
            changed = copy.deepcopy(self.new)
            changed[self.key][field] = self.old[self.key][field] + 1
            with self.subTest(field=field), self.assertRaises(AssertionError):
                check_records(self.old, changed, stack_read_deltas([self.row]))

    def test_duplicate_unused_invalid_and_external_exceptions_fail(self):
        with self.assertRaises(AssertionError):
            stack_read_deltas([self.row, self.row])
        for fields in [dict(delta=0), dict(delta=-2), dict(delta=True), dict(delta=2.0),
                       dict(compiler='vbcc'), dict(mode='unknown'), dict(vector=-1)]:
            with self.subTest(fields=fields), self.assertRaises(AssertionError):
                stack_read_deltas([dict(self.row, **fields)])
        with self.assertRaises(AssertionError):
            check_records(self.old, self.new, stack_read_deltas([dict(self.row, vector=4)]))
        # Supplying no exception still compares the entire external record.
        key = ('unlink', 'optimized', 'vbcc', 0)
        old = {key: dict(self.old[self.key], compiler='vbcc', correct=False)}
        new = copy.deepcopy(old)
        new[key]['cycles'] -= 1
        with self.assertRaises(AssertionError):
            check_records(old, new, {})


if __name__ == '__main__':
    unittest.main()
