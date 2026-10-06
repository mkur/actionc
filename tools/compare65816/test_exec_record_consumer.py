import copy
import unittest
from exec_record_consumer import compare_vectors, compare_stage

class ConsumerChecks(unittest.TestCase):
    def test_next_stage_requires_new_representative_benefits_without_resource_growth(self):
        image=dict(routines=[dict(name='Branch',size=100,fixed_frame=8,local_stack_peak=12,spill_bytes=8)])
        candidate=copy.deepcopy(image);candidate['routines'][0]['size']=99
        placement=[dict(name='Branch',branch_homes=2,resident_entries=2,mixed_edges=1)]
        self.assertEqual(compare_stage(image,candidate,placement,['Branch'])['saved_bytes'],1)
        for field in ('fixed_frame','local_stack_peak','spill_bytes'):
            bad=copy.deepcopy(candidate);bad['routines'][0][field]+=1
            with self.subTest(field=field),self.assertRaises(ValueError): compare_stage(image,bad,placement,['Branch'])
        for bad in (image,dict(routines=[])):
            with self.assertRaises(ValueError): compare_stage(image,bad,placement,['Branch'])
        with self.assertRaises(ValueError): compare_stage(image,candidate,[dict(name='Branch',branch_homes=0)],['Branch'])

    def test_loop_stage_requires_separate_loop_and_call_benefits(self):
        row=lambda name: dict(name=name,size=100,fixed_frame=8,local_stack_peak=12,spill_bytes=8)
        image=dict(routines=[row('Traversal'),row('Caller')]);candidate=copy.deepcopy(image)
        for r in candidate['routines']: r['size']-=1
        placement=[dict(name='Traversal',loop_homes=2,call_segments=0),dict(name='Caller',loop_homes=0,call_segments=1)]
        result=compare_stage(image,candidate,placement,[],stage=5)
        self.assertEqual(result['saved_bytes'],2)
        self.assertEqual(result['loop_benefits'],{'Traversal':1})
        self.assertEqual(result['call_benefits'],{'Caller':1})
        for kind,field in [('loop','loop_homes'),('call','call_segments')]:
            bad=copy.deepcopy(placement)
            for r in bad: r[field]=0
            with self.subTest(kind=kind),self.assertRaises(ValueError):compare_stage(image,candidate,bad,[],stage=5)
        for field in ('fixed_frame','local_stack_peak','spill_bytes'):
            bad=copy.deepcopy(candidate);bad['routines'][0][field]+=1
            with self.subTest(field=field),self.assertRaises(ValueError):compare_stage(image,bad,placement,[],stage=5)

    def measurements(self):
        row = dict(case='Flow', mode='optimized', vector=0, correct=True, args=[1], result=7,
            external_accesses=[[0x21fffe, False, 3], [0x21ffff, True, 7]],
            cycles=100, peak_below_entry_s=12, stack_reads=6, stack_writes=5, dp_reads=2, dp_writes=3)
        return dict(manifest=dict(cases=[dict(id='Flow', vectors=[dict(args=[1], result=7)])]), measurements=[row])

    def test_indexed_stage_requires_new_indexed_benefits_and_retains_aggregate_census(self):
        row=dict(name='Indexed',size=100,fixed_frame=8,local_stack_peak=12,spill_bytes=8)
        image=dict(routines=[row]);candidate=copy.deepcopy(image);candidate['routines'][0]['size']=99
        placement=[dict(name='Indexed',indexed_windows=3,resident_indexed_windows=2,aggregate_windows=0,resident_aggregate_windows=0)]
        result=compare_stage(image,candidate,placement,[],stage=6)
        self.assertEqual(result['indexed_benefits'],{'Indexed':1})
        self.assertEqual(result['resident_indexed_windows'],2)
        self.assertEqual(result['aggregate_windows'],0)
        for field in ('fixed_frame','local_stack_peak','spill_bytes'):
            bad=copy.deepcopy(candidate);bad['routines'][0][field]+=1
            with self.subTest(field=field),self.assertRaises(ValueError):compare_stage(image,bad,placement,[],stage=6)
        for bad in (image,dict(routines=[])):
            with self.assertRaises(ValueError):compare_stage(image,bad,placement,[],stage=6)
        placement[0]['resident_indexed_windows']=0
        with self.assertRaises(ValueError):compare_stage(image,candidate,placement,[],stage=6)

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
