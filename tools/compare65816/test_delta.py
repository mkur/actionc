"""The comparison-specific read accounting must not weaken other checks."""
import copy
import unittest

from delta import PRESERVED, check_records, stack_read_deltas, fused_branch_counts


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


class FusedBranchCounts(unittest.TestCase):
    def setUp(self):
        self.key = ('sum_loop', 'optimized', 'actionc', 0)
        self.row = dict(case='sum_loop', mode='optimized', compiler='actionc', vector=0, count=3)
        record = dict.fromkeys(PRESERVED, 0)
        record.update(compiler='actionc', correct=True, code_bytes=213, cycles=1000,
                      stack_reads=20, stack_writes=10, dp_reads=0, dp_writes=0,
                      dp_touched_offsets=[], errors=[], result=3)
        self.old = {self.key: record}
        self.new = copy.deepcopy(self.old)
        self.new[self.key].update(code_bytes=183, cycles=905, stack_reads=17,
                                  stack_writes=7, fused_branches=3)
        self.counts = fused_branch_counts([self.row])

    def test_strict_default_and_exact_two_sided_reduction(self):
        with self.assertRaises(AssertionError):
            check_records(self.old, self.new, {})
        check_records(self.old, self.new, {}, self.counts)
        for field, value in [('stack_reads', 16), ('stack_reads', 20),
                             ('stack_writes', 8), ('stack_writes', 10),
                             ('stack_writes', 11), ('fused_branches', 2)]:
            changed = copy.deepcopy(self.new)
            changed[self.key][field] = value
            with self.subTest(field=field, value=value), self.assertRaises(AssertionError):
                check_records(self.old, changed, {}, self.counts)
        with self.assertRaises(AssertionError):
            check_records(self.old, self.new, {}, {})  # missing prediction

    def test_other_invariants_and_unlisted_records_stay_strict(self):
        for field in [f for f in PRESERVED if f != 'stack_writes'] + ['dp_reads', 'dp_writes', 'dp_touched_offsets']:
            changed = copy.deepcopy(self.new)
            changed[self.key][field] = 'changed'
            with self.subTest(field=field), self.assertRaises(AssertionError):
                check_records(self.old, changed, {}, self.counts)
        for field in ['cycles', 'code_bytes']:
            changed = copy.deepcopy(self.new)
            changed[self.key][field] = self.old[self.key][field] + 1
            with self.assertRaises(AssertionError):
                check_records(self.old, changed, {}, self.counts)
        key = ('identity', 'raw', 'actionc', 0)
        old = dict(self.old, **{})
        old[key] = dict(self.old[self.key])
        new = copy.deepcopy(self.new)
        new[key] = dict(old[key], fused_branches=0)
        check_records(old, new, {}, self.counts)
        new[key]['stack_writes'] -= 1
        with self.assertRaises(AssertionError):
            check_records(old, new, {}, self.counts)

    def test_invalid_duplicate_stale_and_external_counts_fail(self):
        with self.assertRaises(AssertionError):
            fused_branch_counts([self.row, self.row])
        for fields in [dict(count=0), dict(count=-1), dict(count=True), dict(count=1.0),
                       dict(compiler='vbcc'), dict(mode='bad'), dict(vector=-1), dict(case='')]:
            with self.subTest(fields=fields), self.assertRaises(AssertionError):
                fused_branch_counts([dict(self.row, **fields)])
        with self.assertRaises(AssertionError):
            check_records(self.old, self.new, {}, fused_branch_counts([dict(self.row, vector=9)]))
        with self.assertRaises(AssertionError):
            check_records(self.old, self.new, {self.key: 2}, self.counts)
        key = ('unlink', 'optimized', 'vbcc', 0)
        old = {key: dict(self.old[self.key], compiler='vbcc', correct=False)}
        new = copy.deepcopy(old)
        check_records(old, new, {}, {})
        new[key]['cycles'] -= 1
        with self.assertRaises(AssertionError):
            check_records(old, new, {}, {})



class DirectWordEdgeCounts(unittest.TestCase):
    def setUp(self):
        from delta import direct_word_edge_counts
        self.parse = direct_word_edge_counts
        self.key = ('sum_loop', 'optimized', 'actionc', 3)
        self.row = dict(case='sum_loop', mode='optimized', compiler='actionc', vector=3, count=14)
        r = dict.fromkeys(PRESERVED, 0)
        r.update(compiler='actionc', correct=True, errors=[], result=91, code_bytes=154,
                 cycles=1735, instructions=417, stack_reads=273, stack_writes=216,
                 dp_reads=0, dp_writes=0, dp_touched_offsets=[], fused_branches=14,
                 fused_branch_sites={'1': 14}, word_edges=14, edge_words=14,
                 word_edge_sites={'2': 1, '3': 13})
        self.old = {self.key: r}
        self.new = {self.key: dict(r, code_bytes=146, cycles=1595, instructions=389,
                                 stack_reads=245, stack_writes=188, direct_word_edges=14,
                                 direct_word_edge_sites={'20': 1, '30': 13},
                                 fused_branch_sites={'10': 14}, word_edge_sites={'20': 1, '30': 13})}
        self.counts = self.parse([self.row])

    def check(self, new):
        check_records(self.old, new, {}, direct=self.counts)

    def test_default_stays_strict_and_copy_accounting_is_exact(self):
        with self.assertRaises(AssertionError):
            check_records(self.old, self.new, {})
        self.check(self.new)
        for field in ['stack_reads', 'stack_writes', 'instructions', 'cycles',
                      'direct_word_edges', 'dp_reads', 'dp_writes', 'word_edges',
                      'edge_words', 'fused_branches']:
            for change in [-1, 1]:
                modified = copy.deepcopy(self.new)
                modified[self.key][field] += change
                with self.subTest(field=field, change=change), self.assertRaises(AssertionError):
                    self.check(modified)
        for field in ['fused_branch_sites', 'word_edge_sites', 'direct_word_edge_sites']:
            modified = copy.deepcopy(self.new)
            modified[self.key][field]['extra'] = 1
            with self.subTest(field=field), self.assertRaises(AssertionError):
                self.check(modified)

    def test_contract_and_unlisted_records_remain_strict(self):
        for field in [f for f in PRESERVED if f != 'stack_writes'] + ['dp_touched_offsets']:
            modified = copy.deepcopy(self.new)
            modified[self.key][field] = 'changed'
            with self.subTest(field=field), self.assertRaises(AssertionError):
                self.check(modified)
        key = ('identity', 'raw', 'actionc', 0)
        self.old[key] = copy.deepcopy(self.old[self.key])
        self.new[key] = dict(self.old[key], direct_word_edges=0, direct_word_edge_sites={})
        self.check(self.new)
        for field in ['cycles', 'stack_reads', 'stack_writes', 'code_bytes']:
            modified = copy.deepcopy(self.new)
            modified[key][field] -= 1
            with self.subTest(field=field), self.assertRaises(AssertionError):
                self.check(modified)

    def test_counts_reject_missing_stale_invalid_and_mixed_modes(self):
        for fields in [dict(count=0), dict(count=-1), dict(count=True), dict(count=1.0),
                       dict(compiler='vbcc'), dict(mode='unknown'), dict(vector=-1)]:
            with self.subTest(fields=fields), self.assertRaises(AssertionError):
                self.parse([dict(self.row, **fields)])
        with self.assertRaises(AssertionError): self.parse([self.row, self.row])
        with self.assertRaises(AssertionError): check_records(self.old, self.new, {}, direct={})
        with self.assertRaises(AssertionError): check_records(self.old, self.new, {}, direct=self.parse([dict(self.row, vector=99)]))
        with self.assertRaises(AssertionError): check_records(self.old, self.new, {self.key: 28}, direct=self.counts)
        with self.assertRaises(AssertionError): check_records(self.old, self.new, {}, fused={}, direct=self.counts)
        key = ('unlink', 'optimized', 'vbcc', 0)
        self.old[key] = dict(self.old[self.key], compiler='vbcc', correct=False)
        self.new[key] = copy.deepcopy(self.old[key])
        self.check(self.new)
        self.new[key]['cycles'] -= 1
        with self.assertRaises(AssertionError): self.check(self.new)

class ForwardedWordLoads(unittest.TestCase):
    def setUp(self):
        from delta import forwarded_word_load_counts
        self.parse = forwarded_word_load_counts
        self.key = ('sum_loop', 'optimized', 'actionc', 3)
        self.row = dict(case='sum_loop', mode='optimized', compiler='actionc', vector=3, count=40)
        r = dict.fromkeys(PRESERVED, 0)
        r.update(compiler='actionc', correct=True, code_bytes=146, cycles=1595,
                 instructions=389, stack_reads=245, stack_writes=188, dp_reads=0,
                 dp_writes=0, dp_touched_offsets=[], fused_branches=14,
                 fused_branch_sites={'1':14}, word_edges=14, edge_words=14,
                 word_edge_sites={'2':1,'3':13}, direct_word_edges=14,
                 direct_word_edge_sites={'2':1,'3':13})
        self.old = {self.key:r}
        self.new = {self.key:dict(r, code_bytes=140, cycles=1395, instructions=349,
                                 stack_reads=165, forwarded_word_loads=40,
                                 forwarded_word_load_sites={'10':14,'11':13,'12':13})}
        self.counts = self.parse([self.row])

    def check(self, new):
        check_records(self.old, new, {}, forwarded=self.counts)

    def test_exact_read_instruction_cycle_accounting_and_unchanged_stores(self):
        self.check(self.new)
        with self.assertRaises(AssertionError): check_records(self.old,self.new,{})
        for field in ['stack_reads','stack_writes','instructions','cycles','forwarded_word_loads',
                      'dp_reads','dp_writes','fused_branches','word_edges','edge_words','direct_word_edges']:
            for change in [-1,1]:
                modified = copy.deepcopy(self.new)
                modified[self.key][field] += change
                with self.subTest(field=field,change=change), self.assertRaises(AssertionError): self.check(modified)
        for field in ['fused_branch_sites','word_edge_sites','direct_word_edge_sites','forwarded_word_load_sites']:
            modified=copy.deepcopy(self.new);modified[self.key][field]['extra']=1
            with self.subTest(field=field),self.assertRaises(AssertionError): self.check(modified)
        for field in PRESERVED:
            modified=copy.deepcopy(self.new);modified[self.key][field]='changed'
            with self.subTest(field=field),self.assertRaises(AssertionError): self.check(modified)

    def test_zero_execution_vector_may_share_a_shrunk_body_but_not_changed_traffic(self):
        key=('recursive_sum','raw','actionc',0)
        self.old[key]=copy.deepcopy(self.old[self.key])
        self.new[key]=dict(self.old[key],code_bytes=142,forwarded_word_loads=0,forwarded_word_load_sites={})
        self.check(self.new)
        for field in ['cycles','instructions','stack_reads','stack_writes']:
            modified=copy.deepcopy(self.new);modified[key][field]-=1
            with self.subTest(field=field),self.assertRaises(AssertionError): self.check(modified)

    def test_invalid_missing_duplicate_external_and_mixed_accounting_fail(self):
        for fields in [dict(count=0),dict(count=-1),dict(count=True),dict(count=1.0),dict(compiler='vbcc'),dict(mode='bad'),dict(vector=-1)]:
            with self.subTest(fields=fields),self.assertRaises(AssertionError):self.parse([dict(self.row,**fields)])
        with self.assertRaises(AssertionError):self.parse([self.row,self.row])
        with self.assertRaises(AssertionError):check_records(self.old,self.new,{},forwarded={})
        with self.assertRaises(AssertionError):check_records(self.old,self.new,{},forwarded=self.parse([dict(self.row,vector=99)]))
        for extra in [dict(fused={}),dict(direct={})]:
            with self.assertRaises(AssertionError):check_records(self.old,self.new,{},forwarded=self.counts,**extra)
        with self.assertRaises(AssertionError):check_records(self.old,self.new,{self.key:1},forwarded=self.counts)
        key=('unlink','optimized','vbcc',0)
        self.old[key]=dict(self.old[self.key],compiler='vbcc',correct=False)
        self.new[key]=copy.deepcopy(self.old[key]);self.check(self.new)
        self.new[key]['cycles']-=1
        with self.assertRaises(AssertionError):self.check(self.new)

if __name__ == '__main__':
    unittest.main()
