import copy
import unittest
from check_acyclic_edges import transform


class AcyclicProof(unittest.TestCase):
    def fixture(self):
        # 4 -> 2, immediate -> 4, followed by a backward jump.
        before=[(0,b'\xa3\x04'),(2,b'\x83\x08'),(4,b'\xa9\x5a\xa5'),(7,b'\x83\x0c'),
                (9,b'\xa3\x08'),(11,b'\x83\x02'),(13,b'\xa3\x0c'),(15,b'\x83\x04'),(17,b'\x5c\x00\x00\x00')]
        after=[(0,b'\xa3\x04'),(2,b'\x83\x02'),(4,b'\xa9\x5a\xa5'),(7,b'\x83\x04'),(9,b'\x5c\x00\x00\x00')]
        moves=[dict(source=dict(kind='stack',offset=4,width=2,word_operand=True),destination=dict(kind='stack',offset=2,width=2),staging=dict(kind='stack',offset=8,width=4),width=2),
               dict(source=dict(kind='immediate',value=0xa55a,width=2,word_operand=True),destination=dict(kind='stack',offset=4,width=2),staging=dict(kind='stack',offset=12,width=4),width=2)]
        e=dict(block_pc=0,transfer_pc=17,target_pc=0,fallthrough=False,form='staged_word',moves=moves,
               copy_pcs=[p for p,_ in before[:-1]],analysis=dict(graph='ordered_acyclic'))
        return before,after,e

    def test_interleaving_keeps_sources_destinations_and_jump(self):
        before,after,e=self.fixture();mapping,removed=transform(before,after,[e])
        self.assertEqual(removed,{2,7,9,13})
        self.assertEqual(mapping,{0:0,11:2,4:4,15:7,17:9})

    def test_wrong_source_destination_missing_site_or_cyclic_claim_fails(self):
        before,after,e=self.fixture()
        for i in range(len(after)):
            bad=list(after);bad[i]=(bad[i][0],b'\xea')
            with self.assertRaises(AssertionError):transform(before,bad,[e])
        for field,value in [('copy_pcs',[0]),('analysis',dict(graph='cyclic'))]:
            bad=copy.deepcopy(e);bad[field]=value
            with self.assertRaises(AssertionError):transform(before,after,[bad])


if __name__=='__main__':unittest.main()
