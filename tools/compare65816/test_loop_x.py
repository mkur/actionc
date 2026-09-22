import copy
import json
from pathlib import Path
import unittest
from loop_x import candidate, sites, transform, measurement, ROOT
from check_selective_staging import all_instructions


class LoopX(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        facts=json.loads((ROOT/'docs/benchmarks/65816-register-inventory/facts.json').read_text())
        cls.r=next(b for b in facts['builds'] if (b['case'],b['mode'])==('loop_rotation','optimized'))['routines'][0]

    def test_typed_candidate_and_rejections(self):
        self.assertEqual(candidate(self.r)['param'],18)
        changes=[lambda r:r['temp_homes'][4]['type'].update(signed=True),
                 lambda r:r['blocks'][1]['ops'][0]['values'][1].update(value=65535),
                 lambda r:r['blocks'][1]['ops'][0].update(signed=True),
                 lambda r:r['blocks'][1]['ops'][0].update(predicate='eq'),
                 lambda r:r['blocks'][2]['ops'][0]['uses'].append(18),
                 lambda r:r['blocks'][3]['terminator_uses'].append(18),
                 lambda r:r['edges'].append(copy.deepcopy(r['edges'][0])),
                 lambda r:r['blocks'][2]['ops'][0]['effects'].update(barrier=True)]
        for change in changes:
            r=copy.deepcopy(self.r);change(r)
            self.assertTrue(candidate(r)['rejections'])

    def test_forecast_arithmetic_and_source_authentication(self):
        # Reconstruct the committed old stream from the portable instruction inventory.
        frozen=json.loads((ROOT/'docs/benchmarks/65816-register-inventory/inventory.json').read_text())
        b=next(b for b in frozen['builds'] if (b['case'],b['mode'])==('loop_rotation','optimized'))
        routines=b['routines'];code=[byte for r in routines for i in r['instructions'] for byte in bytes.fromhex(i['bytes'])]
        image=dict(format='actionc-65816-image',version=3,abi='action65816.native.v1',entry=routines[-1]['address'],
                   segments=[dict(address=65536,bytes=code,executable=True)],
                   routines=[dict(address=r['address'],size=r['size']) for r in routines])
        t=sites(candidate(self.r),dict(all_instructions(image)))
        after,_,remap,refresh=transform(image,t)
        self.assertEqual(after['routines'][0]['size'],129)
        self.assertEqual(after['segments'][0]['bytes'][0x3d:0x43],[0xaa,0xe0,8,0,0x90,4])
        for row in b['measurements']:
            old=dict(row,dp_reads=102);new=measurement(old,t,remap,refresh)
            self.assertEqual((new['cycles'],new['instructions'],new['dp_reads']),(759,218,68))
        bad=copy.deepcopy(image);bad['segments'][0]['bytes'][0x57]=0x24
        with self.assertRaises(AssertionError):transform(bad,t)
        with self.assertRaises(AssertionError):remap(t['removed_pc'])


if __name__=='__main__':unittest.main()
