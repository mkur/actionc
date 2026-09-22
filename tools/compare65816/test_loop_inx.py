import copy
import json
import unittest
from loop_inx import select, transform, measurement, ROOT
from post_x_inventory import movement


class Inx(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.inventory=json.loads((ROOT/'docs/benchmarks/65816-post-x-inventory/inventory.json').read_text())
        cls.facts=json.loads((ROOT/'docs/benchmarks/65816-post-x-inventory/facts.json').read_text())
        cls.build=next(b for b in cls.inventory['builds'] if (b['case'],b['mode'])==('loop_rotation','optimized'))
        cls.r=next(b for b in cls.facts['builds'] if (b['case'],b['mode'])==('loop_rotation','optimized'))['routines'][0]
        routines=cls.build['routines'];cls.ins={i['pc']:bytes.fromhex(i['bytes']) for r in routines for i in r['instructions']}
        cls.image=dict(format='actionc-65816-image',version=3,abi='action65816.native.v1',entry=routines[-1]['address'],
            segments=[dict(address=65536,bytes=list(b''.join(cls.ins.values())),executable=True)],
            routines=[dict(address=r['address'],size=r['size']) for r in routines])

    def test_exact_transform_and_counter_forecast(self):
        t=select(self.r,self.ins);after,ins,remap,_=transform(self.image,t)
        self.assertEqual(after['routines'][0]['size'],126)
        self.assertEqual(dict(ins)[t['update_pc']],b'\xe8')
        self.assertEqual(dict(ins)[t['update_pc']+1],b'\x8a')
        old=json.loads((ROOT/'docs/benchmarks/65816-loop-x/frozen.json').read_text())
        b=next(b for b in old['builds'] if b['selected'])
        for projection in b['projections']:
            r=projection['forecast'];n=measurement(r,t,remap)
            self.assertEqual((n['cycles'],n['instructions'],n['dp_reads'],n['dp_writes']),(735,210,68,104))
            self.assertEqual((n['x_forwarded_loads'],n['x_increment_updates']),(0,8))
        with self.assertRaises(AssertionError):remap(t['removed_pc'])
        bad=copy.deepcopy(self.image);bad['segments'][0]['bytes'][t['update_pc']-65536+3]=2
        with self.assertRaises(AssertionError):transform(bad,t)

    def test_admission_authenticates_current_typed_update(self):
        t=select(self.r,self.ins)
        for pc in (t['update_pc'],t['update_pc']+1,t['update_pc']+2,t['compare_range'][0]):
            bad=dict(self.ins);bad[pc]=bytes([bad[pc][0]^1])+bad[pc][1:]
            with self.assertRaises(AssertionError):select(self.r,bad)
        r=copy.deepcopy(self.r);r['blocks'][2]['ops'][3]['values'][1]['value']=2
        self.assertTrue(select(r,self.ins)['rejections'])
        r=copy.deepcopy(self.r);o=copy.deepcopy(r['blocks'][1]['ops'][0]);o['index']=4;o['uses']=[];o['values'][0]={'kind':'immediate','value':0,'width':2,'word_operand':True};r['blocks'][2]['ops'].append(o)
        self.assertEqual(select(r,self.ins)['rejections'],['pending_internal_dispatch'])

    def test_all_build_movement_rollup_preserves_required_traffic(self):
        result=movement(self.inventory)
        self.assertEqual(len(result['builds']),28)
        rotation=next(b for b in result['builds'] if (b['case'],b['mode'])==('loop_rotation','optimized'))
        self.assertEqual(rotation['records'][2]['retained_word_stores'],68)
        self.assertEqual(rotation['records'][2]['goto_backedge_word_loads'],32)
        self.assertEqual(rotation['records'][2]['traffic'],{'stack:edge_staging:read':16,'stack:edge_staging:write':16})


if __name__=='__main__':unittest.main()
