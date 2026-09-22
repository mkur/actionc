import unittest
from check_parameter_forwarding import transform, measurement


class ParameterForwarding(unittest.TestCase):
    def fixture(self):
        # Two stores survive; absolute and relative targets after the reload move.
        code=bytes.fromhex('a30a8302a30a830c d002 5c100001 a90200 6b')
        return dict(format="actionc-65816-image",version=3,abi="action65816.native.v1",entry=0x10000,segments=[dict(address=0x10000,bytes=list(code),executable=True)],routines=[dict(address=0x10000,size=len(code),calls=[dict(outgoing=3,transfer_peak=3)])]),dict(reload_pc=0x10004,parameter_body_displacement=10,producer_pc=0x10000,window_end=0x10008,before_bytes='a30a8302a30a830c',forecast_bytes='a30a8302830c')

    def test_only_load_is_removed_and_both_stores_and_targets_remain(self):
        image,site=self.fixture()
        out,ins,remap=transform(image,site)
        self.assertEqual(bytes(out['segments'][0]['bytes'])[:6],bytes.fromhex('a30a8302830c'))
        self.assertEqual(out['routines'][0]['size'],image['routines'][0]['size']-2)
        self.assertEqual(dict(ins)[0x10008],bytes.fromhex('5c0e0001'))
        self.assertEqual(remap(0x20000),0x20000)

    def test_entry_into_removed_load_rejects(self):
        image,site=self.fixture()
        image['entry']=site['reload_pc']
        with self.assertRaises(AssertionError):transform(image,site)

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
        self.assertEqual(out['parameter_forwarded_loads'],8)
        self.assertEqual(out['instruction_sites'],{'100':8})
        self.assertEqual(measurement(old,None,lambda p:p)['cycles'],956)

    def test_zero_execution_keeps_static_saving_and_no_dynamic_saving(self):
        old=dict(code_bytes=208,cycles=100,instructions=32,stack_reads=9,stack_writes=4,instruction_sites={})
        out=measurement(old,dict(reload_pc=100),lambda p:p)
        self.assertEqual(out['code_bytes'],206)
        self.assertEqual(out['cycles'],100)
        self.assertEqual(out['parameter_forwarded_load_sites'],{})

    def test_bridge_keeps_all_three_stores(self):
        image,site=self.fixture()
        image['segments'][0]['bytes'][4:4]=[0x83,6]
        image['routines'][0]['size']+=2
        # No control-transfer instructions needed for this independent shape.
        image['segments'][0]['bytes']=list(bytes.fromhex('a30a83028306a30a830c6b'))
        image['routines'][0]['size']=11
        site.update(reload_pc=0x10006,window_end=0x1000a,before_bytes='a30a83028306a30a830c',forecast_bytes='a30a83028306830c')
        out,_,_=transform(image,site)
        self.assertEqual(bytes(out['segments'][0]['bytes']).hex(),'a30a83028306830c6b')

if __name__=='__main__':unittest.main()
