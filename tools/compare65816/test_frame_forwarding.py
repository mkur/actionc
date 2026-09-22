import unittest
from check_frame_forwarding import transform, measurement


class FrameForwarding(unittest.TestCase):
    def fixture(self):
        # Two stores survive; absolute and relative targets after the reload move.
        code=bytes.fromhex('a30a8302a302830c d002 5c100001 a90200 6b')
        return dict(format="actionc-65816-image",version=3,abi="action65816.native.v1",entry=0x10000,segments=[dict(address=0x10000,bytes=list(code),executable=True)],routines=[dict(address=0x10000,size=len(code),calls=[])]),dict(reload_pc=0x10004,source_slot=2,before_bytes='a30a8302a302830c',after_bytes='a30a8302830c')

    def test_only_load_is_removed_and_both_stores_and_targets_remain(self):
        image,site=self.fixture()
        out,ins,remap=transform(image,site)
        self.assertEqual(bytes(out['segments'][0]['bytes'])[:6],bytes.fromhex('a30a8302830c'))
        self.assertEqual(out['routines'][0]['size'],image['routines'][0]['size']-2)
        self.assertEqual(dict(ins)[0x10008],bytes.fromhex('5c0e0001'))
        self.assertEqual(remap(0x20000),0x20000)

    def test_bad_slot_or_missing_store_rejects(self):
        for offset in [1,2,3,4,5,6,7]:
            image,site=self.fixture()
            image['segments'][0]['bytes'][offset]^=1
            with self.assertRaises((AssertionError,ValueError,KeyError)):transform(image,site)

    def test_transfer_into_deleted_load_rejects(self):
        image,site=self.fixture()
        image['segments'][0]['bytes'][11:14]=[4,0,1]
        with self.assertRaises((AssertionError,ValueError,KeyError)):transform(image,site)

    def test_metrics_preserve_old_forwarding_and_stores(self):
        old=dict(code_bytes=140,cycles=956,instructions=230,stack_reads=143,stack_writes=140,
                 instruction_sites={'100':8,'102':8},forwarded_word_loads=8,forwarded_word_load_sites={'102':8})
        out=measurement(old,dict(reload_pc=100),lambda p:p-2 if p>=102 else p)
        self.assertEqual((out['cycles'],out['instructions'],out['stack_reads'],out['stack_writes']),(916,222,127,140))
        self.assertEqual(out['forwarded_word_loads'],8)
        self.assertEqual(out['frame_forwarded_loads'],8)
        self.assertEqual(out['instruction_sites'],{'100':8})
        self.assertEqual(measurement(old,None,lambda p:p)['cycles'],956)

if __name__=='__main__':unittest.main()
