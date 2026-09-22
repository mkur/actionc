import copy
import json
from pathlib import Path
import unittest
from check_selective_staging import address, instructions, measurements, relocate


class SelectiveProof(unittest.TestCase):
    def setUp(self):
        self.frozen = json.loads((Path(__file__).resolve().parents[2]/'docs/benchmarks/65816-selective-staging/baseline.json').read_text())
        self.site = self.frozen['selected_site']
        s = self.site['first_copy_pc']
        w = bytes.fromhex(self.site['before_copy_bytes'])
        self.before = [(0x10015,bytes.fromhex('e91400')),(0x10026,bytes.fromhex('a91400')),
                       (0x1002e,bytes.fromhex('a318'))] + [(s+i,w[i:i+2]) for i in range(0,len(w),2)] + [(0x10080,bytes.fromhex('5c450001')),(0x1008e,bytes.fromhex('691400'))]

    def test_only_forecast_transform_is_admitted(self):
        expected,mapping,removed,_ = instructions(self.before,self.site,[(0x10000,0x10094)])
        self.assertEqual(removed,{0x1006a,0x10072,0x10074,0x1007c})
        self.assertEqual(mapping[0x1006c],0x10068)
        self.assertEqual(mapping[0x10068],0x1006c)
        self.assertEqual(expected[-1],(0x10086,bytes.fromhex('691000')))
        for i in range(len(self.before)):
            pc, b = self.before[i]
            if pc in [0x10080]:
                continue
            bad = self.before.copy(); bad[i] = (pc,b[:-1]+bytes([b[-1]^1]))
            with self.assertRaises(AssertionError):
                instructions(bad,self.site,[(0x10000,0x10094)])
        with self.assertRaises(AssertionError): address(0x1006c,self.site)

    def test_long_branch_and_per_references_remap(self):
        remap = lambda p: p-8 if p >= 0x180 else p
        self.assertEqual(relocate(0x100,bytes.fromhex('5c900100'),0x100,remap),bytes.fromhex('5c880100'))
        self.assertEqual(relocate(0x178,b'\xd0\x16',0x178,remap),b'\xd0\x0e')
        self.assertEqual(relocate(0x178,b'\x62\x14\x00',0x178,remap),b'\x62\x0c\x00')
        self.assertEqual(relocate(0x190,b'\x62\x6c\xff',0x188,remap),b'\x62\x74\xff')

    def test_measurements_preserve_every_undeclared_field(self):
        _,mapping,removed,remap = instructions(self.before,self.site,[(0x10000,0x10094)])
        f = self.frozen['forecasts'][0]
        old = dict(f['before'],vector=0,correct=True,dp_reads=0,stack_check_cycles=36,
                   instruction_sites={str(pc):8 for pc,_ in self.before},word_edge_sites={'65640':8})
        saved = copy.deepcopy(old)
        new = measurements(old,self.site,{0:f},mapping,removed,remap)
        self.assertEqual(old,saved)
        self.assertEqual(new['cycles'],956)
        self.assertEqual(new['selective_word_edge_sites'],{'65640':8})
        self.assertEqual(new['word_edge_sites'],old['word_edge_sites'])
        self.assertEqual(new['stack_check_cycles'],36)
        self.assertEqual(new['dp_reads'],0)
        with self.assertRaises(AssertionError):
            measurements(dict(old,stack_reads=174),self.site,{0:f},mapping,removed,remap)


if __name__ == '__main__': unittest.main()
