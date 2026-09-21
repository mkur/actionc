import unittest
from check_control_flow import select, transform


class ControlFlowProof(unittest.TestCase):
    def test_width_deletion_remaps_incoming_jump_and_retains_other_rep(self):
        before = [(0,b'\x5c\x04\x00\x00'),(4,b'\xc2\x20'),(6,b'\xc2\x20'),(8,b'\x6b')]
        after = [(0,b'\x5c\x04\x00\x00'),(4,b'\xc2\x20'),(6,b'\x6b')]
        mapping, removed = transform(before,after,[dict(pc=4,selected=True)],'3a')
        self.assertEqual(removed,{4});self.assertEqual(mapping[4],4)
        with self.assertRaises(AssertionError):
            transform(before,after,[dict(pc=0,selected=True)],'3a')

    def test_adjacent_jump_and_rejection(self):
        before = [(0,b'\x5c\x04\x00\x00'),(4,b'\x6b')]
        transform(before,[(0,b'\x6b')],[dict(pc=0,target=4,selected=True)],'3b')
        for bad in (dict(pc=0,target=5,selected=True),dict(pc=1,target=5,selected=True)):
            with self.assertRaises(AssertionError):transform(before,[(0,b'\x6b')],[bad],'3b')

    def test_short_predicate_target_and_unrelated_changes(self):
        before=[(0,b'\xf0\x04'),(2,b'\x5c\x07\x00\x00'),(6,b'\xea'),(7,b'\x6b')]
        sites=[dict(pc=0,target=7,predicate=0xd0,selected=True)]
        after=[(0,b'\xd0\x01'),(2,b'\xea'),(3,b'\x6b')]
        transform(before,after,sites,'3c')
        for first in (b'\xf0\x01',b'\xd0\x02'):
            with self.assertRaises(AssertionError):transform(before,[(0,first),*after[1:]],sites,'3c')
        with self.assertRaises(AssertionError):transform(before,[*after[:1],(2,b'\x18'),*after[2:]],sites,'3c')
        with self.assertRaises(AssertionError):transform(before,after,sites+sites,'3c')

    def test_branch_range_and_candidate_shrink(self):
        for delta in (-129,-128,-127,0,126,127,128):
            pc=200;target=pc+2+delta
            if target>=pc+2:target+=4
            site=dict(slice='3c',pc=pc,target=target)
            selected=select([site],'3c')[0]['selected']
            self.assertEqual(selected,-128<=delta<=127,delta)

    def test_cascading_shrinks(self):
        sites=[dict(slice='3c',pc=0,target=136),dict(slice='3c',pc=20,target=30)]
        self.assertTrue(all(s['selected'] for s in select(sites,'3c')))

    def test_short_branch_cannot_have_independent_entry_in_removed_jml(self):
        before=[(0,b'\xd0\x04'),(2,b'\x5c\x06\x00\x00'),(6,b'\x5c\x02\x00\x00')]
        after=[(0,b'\xf0\x00'),(2,b'\x5c\x02\x00\x00')]
        with self.assertRaises(AssertionError):
            transform(before,after,[dict(pc=0,target=6,predicate=0xf0,selected=True)],'3c')


if __name__=='__main__':unittest.main()
