import unittest
from check_edge_coalescing import transform,measurement

class EdgeCoalescing(unittest.TestCase):
    def fixture(self):
        code=bytes.fromhex('8306 8308 a306 830a a308 8306 a90000 8308 5c150001 6b')
        site=dict(routine=0,home_changes=[dict(temp=0,old_offset=6,new_offset=10),dict(temp=2,old_offset=8,new_offset=6)],operand_patches=[dict(pc=0x10000,before='8306',after='830a'),dict(pc=0x10002,before='8308',after='8306')],removed_pcs=list(range(0x10004,0x1000c,2)),old_transfer_pc=0x10011,before_copy_bytes='a306830aa3088306a900008308',after_copy_bytes='a900008308')
        image=dict(format="actionc-65816-image",version=3,abi="action65816.native.v1",entry=0x10000,segments=[dict(address=0x10000,bytes=list(code),executable=True)],routines=[dict(id=0,address=0x10000,size=len(code),calls=[],temporaries=[dict(id=0,size=2,home=dict(kind='stack',displacement=6)),dict(id=2,size=2,home=dict(kind='stack',displacement=8))])])
        return image,site
    def test_frozen_transform_patches_only_named_homes_and_deletes_copies(self):
        image,site=self.fixture();out,ins,remap=transform(image,site)
        self.assertEqual(bytes(out['segments'][0]['bytes'])[:9].hex(),'830a8306a900008308')
        self.assertEqual(out['routines'][0]['temporaries'][0]['home']['displacement'],10)
        self.assertEqual(remap(0x20000),0x20000)
        self.assertEqual(dict(ins)[0x10009].hex(),'5c0d0001')
    def test_mutated_producers_or_copy_operands_reject(self):
        for at in range(17):
            image,site=self.fixture();image['segments'][0]['bytes'][at]^=1
            with self.assertRaises((AssertionError,KeyError,ValueError)):transform(image,site)
    def test_removed_targets_and_incorrect_maps_reject(self):
        for case in range(3):
            image,site=self.fixture()
            if case==0:image['entry']=0x10004
            elif case==1:image['segments'][0]['bytes'][18:21]=[4,0,1]
            else:image['routines'][0]['temporaries'][0]['home']['displacement']=8
            with self.assertRaises(AssertionError):transform(image,site)
    def test_metrics_keep_logical_edge_counts_and_existing_forwarding(self):
        old=dict(code_bytes=138,cycles=916,instructions=222,stack_reads=127,stack_writes=140,acyclic_edge_words=3,forwarded_word_loads=4,instruction_sites={'100':1,'102':1,'104':1,'106':1,'108':1},word_edge_sites={'100':1})
        out=measurement(old,dict(removed_pcs=[100,102,104,106]),lambda p:p-8 if p>=108 else p)
        self.assertEqual([out[k] for k in ('code_bytes','cycles','instructions','stack_reads','stack_writes')],[130,896,218,123,136])
        self.assertEqual(out['instruction_sites'],{'100':1});self.assertEqual(out['word_edge_sites'],{'100':1})
        self.assertEqual(out['coalesced_word_copies'],2);self.assertEqual(out['acyclic_edge_words'],3);self.assertEqual(out['forwarded_word_loads'],4)
        self.assertEqual(measurement(old,None,lambda p:p)['cycles'],916)

if __name__=='__main__':unittest.main()
