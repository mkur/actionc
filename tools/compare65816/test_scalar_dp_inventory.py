import copy
import json
from pathlib import Path
import unittest
from inventory_scalar_dp import admission, trial, freeze_routine, transform, measurements
from check_selective_staging import all_instructions

ROOT=Path(__file__).resolve().parents[2]
FACTS=ROOT/'docs/benchmarks/65816-scalar-dp-inventory/facts.json'

class ScalarInventory(unittest.TestCase):
    def routine(self,case='sum_loop',mode='optimized'):
        f=json.loads(FACTS.read_text())
        return next(b for b in f['builds'] if (b['case'],b['mode'])==(case,mode))['routines'][0]

    def test_typed_rejections_and_closed_home_classes(self):
        r=self.routine();t=trial(r)
        self.assertEqual(t['new_extent'],6)
        self.assertEqual([(c['stack'],c['dp']) for c in t['classes']],[(6,32),(8,34),(10,36)])
        for mutate in [lambda r:r['fixed_objects'][0].update(addressable=True),
                       lambda r:r['parameters'][0].update(width=3),
                       lambda r:r['temp_homes'][0]['type'].update(pointer=True),
                       lambda r:r['blocks'][1]['ops'][0]['effects'].update(calls=True),
                       lambda r:r['blocks'][1]['ops'][0].update(safe_direct_memory=False),
                       lambda r:r['blocks'][1].update(parameter_widths=[1])]:
            bad=copy.deepcopy(r);mutate(bad);self.assertTrue(admission(bad)[0])
        self.assertIn('operation:cast',admission(self.routine(mode='raw'))[0])

    def test_cycles_keep_stack_capture_and_existing_equalities(self):
        r=self.routine('loop_rotation');t=trial(r)
        self.assertEqual(t['new_extent'],8)
        self.assertEqual(t['staging'],[dict(old=14,new=6,width=2)])
        self.assertTrue(any({0,16}<=set(c['temps']) for c in t['classes']))
        self.assertTrue(any({2,17}<=set(c['temps']) for c in t['classes']))
        bad=copy.deepcopy(r)
        for home in bad['temp_homes']:
            if home['home']['width']==2:home['home']['offset']=6
        with self.assertRaises(AssertionError):trial(bad)

    def test_transform_namespaces_and_relocation(self):
        # One zero-frame teardown, followed by an uncounted routine; unrelated
        # data at the same numeric offset is not a code position.
        code=bytes.fromhex('8520 a8 3b 18 690400 1b 98 6b 5c000001 6b')
        image=dict(format="actionc-65816-image",version=3,abi="action65816.native.v1",entry=0x1000b,segments=[dict(address=0x10000,bytes=list(code),executable=True)],
                   routines=[dict(id=0,address=0x10000,size=11,fixed_frame=4,spill_bytes=4,local_stack_peak=4,calls=[],arguments=[dict(body_displacement=8)],temporaries=[dict(id=0,size=2,home=dict(kind='stack',displacement=2))]),
                             dict(id=1,address=0x1000b,size=5)])
        t=dict(routine=0,rejections=[],old_extent=4,new_extent=0,classes=[dict(stack=2,dp=32,temps=[0])],patches=[],removed_pcs=[0x10002,0x10003,0x10004,0x10005,0x10008,0x10009])
        out,ins,remap,_=transform(image,[t])
        self.assertEqual(out['entry'],0x10003)
        self.assertEqual(bytes(out['segments'][0]['bytes']).hex(),'85206b5c0000016b')
        self.assertEqual(out['routines'][0]['fixed_frame'],0)
        self.assertEqual(remap(0x20000),0x20000)
        self.assertEqual(image['routines'][0]['fixed_frame'],4)

    def test_bad_patch_is_rejected_before_transform(self):
        image=dict(format="actionc-65816-image",version=3,abi="action65816.native.v1",entry=65536,segments=[dict(address=65536,bytes=[0x6b],executable=True)],routines=[])
        with self.assertRaises(AssertionError):
            transform(image,[dict(routine=0,rejections=[],patches=[dict(pc=65536,before='a302',after='a520')],removed_pcs=[])])

if __name__=='__main__':unittest.main()
