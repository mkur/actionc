import copy
import unittest
from check_staging_reservations import instructions, routine


class StagingProof(unittest.TestCase):
    def test_only_frozen_operands_can_change(self):
        before = [(0, b'\xe9\x10\x00'), (3, b'\xa3\x14'), (5,b'\x83\x08')]
        after = [(0, b'\xe9\x0c\x00'), (3, b'\xa3\x10'), (5,b'\x83\x08')]
        patches = [dict(pc=0, before='e91000', after='e90c00'), dict(pc=3,before='a314',after='a310')]
        instructions(before,after,patches)
        for bad in [after[:-1], list(reversed(after)), after[:-1]+[(5,b'\x83\x06')], after[:-1]+[(6,b'\x83\x08')]]:
            with self.assertRaises(AssertionError): instructions(before,bad,patches)
        for bad in [patches[:-1], patches+patches[:1], [dict(pc=0,before='e91200',after='e90c00')], [dict(pc=0,before='e91000',after='a90c00')]]:
            with self.assertRaises(AssertionError): instructions(before,after,bad)

    def test_only_frame_accounting_and_incoming_displacements_change(self):
        before = dict(id=0,fixed_frame=16,spill_bytes=12,local_stack_peak=16,arguments=[dict(offset=0,body_displacement=20,size=2,alignment=2)],calls=[],whole_task_stack_bound=None,temporaries=[dict(id=1,home=dict(kind='stack',displacement=6),size=2)])
        original = copy.deepcopy(before)
        c = dict(routine=0,old_extent=16,new_extent=12)
        after = routine(before,c)
        self.assertEqual(before,original)
        self.assertEqual((after['fixed_frame'],after['spill_bytes'],after['local_stack_peak']),(12,8,12))
        self.assertEqual(after['arguments'][0]['body_displacement'],16)
        self.assertEqual(after['temporaries'],before['temporaries'])
        self.assertEqual(routine(before,None),before)
        for bad in [dict(c,old_extent=18),dict(c,new_extent=17),dict(c,new_extent=13)]:
            with self.assertRaises(AssertionError):routine(before,bad)


if __name__ == '__main__': unittest.main()
