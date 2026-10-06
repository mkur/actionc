import copy
import unittest
from exec_record_consumer import compare_vectors

class ConsumerChecks(unittest.TestCase):
    def measurements(self):
        row = dict(case='Flow', mode='optimized', vector=0, correct=True, args=[1], result=7,
            external_accesses=[[0x21fffe, False, 3], [0x21ffff, True, 7]],
            cycles=100, peak_below_entry_s=12, stack_reads=6, stack_writes=5, dp_reads=2, dp_writes=3)
        return dict(manifest=dict(cases=[dict(id='Flow', vectors=[dict(args=[1], result=7)])]), measurements=[row])

    def test_cost_changes_require_identical_independent_oracles_and_accesses(self):
        before=self.measurements();after=copy.deepcopy(before)
        after['measurements'][0].update(cycles=95, stack_reads=4)
        result=compare_vectors(before,after)
        self.assertEqual(result['after']['private'],14)
        self.assertEqual(result['before']['private'],16)

    def test_external_order_address_width_and_payload_are_checked(self):
        for change in ('order','address','width','payload','missing'):
            before=self.measurements();after=copy.deepcopy(before)
            trace=after['measurements'][0]['external_accesses']
            if change=='order': trace.reverse()
            elif change=='address': trace[0][0]+=1
            elif change=='width': trace.append(trace[0])
            elif change=='payload': trace[1][2]^=1
            else: del after['measurements'][0]['external_accesses']
            with self.subTest(change=change),self.assertRaises(ValueError): compare_vectors(before,after)

    def test_stack_cost_oracle_and_census_regressions_are_rejected(self):
        for field,value in [('peak_below_entry_s',13),('cycles',106),('correct',False),('result',8),('args',[2]),('vector',1)]:
            before=self.measurements();after=copy.deepcopy(before)
            after['measurements'][0][field]=value
            with self.subTest(field=field),self.assertRaises(ValueError): compare_vectors(before,after)

    def test_changed_fixture_is_not_a_candidate_comparison(self):
        before=self.measurements();after=copy.deepcopy(before)
        after['manifest']['cases'][0]['vectors'][0]['result']=8
        with self.assertRaises(ValueError): compare_vectors(before,after)

    def test_duplicate_measurement_cannot_disappear_from_the_census(self):
        before=self.measurements();after=copy.deepcopy(before)
        after['measurements'].append(copy.deepcopy(after['measurements'][0]))
        with self.assertRaises(ValueError): compare_vectors(before,after)

if __name__=='__main__':unittest.main()
