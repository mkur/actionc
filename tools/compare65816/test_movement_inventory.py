import copy
import unittest
from inventory_movements import coalescing, edge_code, interference, reloads


def stack(at,temp=None):
    d=dict(kind='stack',offset=at,width=2,word_operand=True)
    if temp is not None:d['temp']=temp
    return d


def move(s,d,temp):
    return dict(source=s,destination=stack(d),destination_temp=temp,width=2,coalescing_attempts=[])


def routine(code,ops,labels=()):
    ins={};pc=100
    for c in code:ins[pc]=bytes(c);pc+=len(c)
    return dict(blocks=[dict(id=0,ops=ops)],edges=[],labels=list(labels)),ins


def operation(i,start,end,**kwargs):
    return dict(index=i,range=[start,end],definition=None,uses=[],producer=None,consumer=None,**kwargs)


class MovementInventory(unittest.TestCase):
    def test_selective_and_direct_edges_decode_actual_capture_pool(self):
        moves=[move(stack(6,0),10,3),move(stack(10,1),6,4),move(stack(12,2),8,5)]
        e=dict(moves=moves,block_pc=100,transfer_pc=116,target_pc=80,fallthrough=False)
        ins={100:b'\xa3\x0a',102:b'\x83\x0e',104:b'\xa3\x06',106:b'\x83\x0a',108:b'\xa3\x0e',110:b'\x83\x06',112:b'\xa3\x0c',114:b'\x83\x08',116:b'\x5c\x50\0\0'}
        result=edge_code(e,[dict(offset=14,width=2)],ins)
        self.assertEqual(result['captures'],[1]);self.assertEqual(result['form'],'selective_word')
        for at in ins:
            bad=ins.copy();bad[at]=b'\0'+bad[at][1:]
            with self.assertRaises(AssertionError):edge_code(e,[dict(offset=14,width=2)],bad)
        with self.assertRaises(AssertionError):edge_code(e,[dict(offset=6,width=2)],ins)
        direct=dict(e,moves=[move(stack(4,0),2,1)],transfer_pc=104,target_pc=104,fallthrough=True)
        self.assertEqual(edge_code(direct,[],{100:b'\xa3\4',102:b'\x83\2'})['form'],'direct_word')

    def test_reordered_direct_tail_retains_final_a_reload(self):
        e=dict(moves=[move(stack(6,0),2,2),move(stack(2,1),4,3)],block_pc=100,transfer_pc=110,target_pc=110,fallthrough=True)
        ins={100:b'\xa3\2',102:b'\x83\4',104:b'\xa3\6',106:b'\x83\2',108:b'\xa3\4'}
        result=edge_code(e,[],ins)
        self.assertEqual(result['reload'],108)
        self.assertEqual([a['move'] for a in result['assignments']],[1,0])
        with self.assertRaises(AssertionError):edge_code(e,[],ins|{108:b'\xa3\2'})

    def test_byte_fallback_uses_actual_width_and_packed_slot(self):
        e=dict(moves=[dict(source=dict(kind='immediate',value=7,width=1,word_operand=False),destination=dict(kind='stack',offset=2,width=1),width=1)],block_pc=100,transfer_pc=110,target_pc=110,fallthrough=True)
        ins={100:b'\xa9\7',102:b'\x83\4',104:b'\xa3\4',106:b'\x83\2',108:b'\xc2\x20'}
        self.assertEqual(edge_code(e,[dict(offset=4,width=1)],ins)['form'],'complete_byte')

    def test_byte_identity_allows_disjoint_store_and_clc_sec(self):
        op=operation(0,100,111,kind='binary');op['producer']=0;op['consumer']=stack(2,0)
        r,ins=routine([[0xa3,2],[0x83,4],[0x18],[0x38],[0x83,6],[0xa3,2],[0x6b]],[op])
        found=reloads(r,ins)
        self.assertEqual(found[-1]['classification'],'broader_temp_forwarding')
        self.assertEqual(found[-1]['conditional_saving']['cycles'],5)
        self.assertEqual(found[-1]['window'],[[pc,c.hex()] for pc,c in ins.items() if pc<=108])

    def test_flags_alias_calls_labels_modes_and_partial_stores_block_claims(self):
        for middle,reason in [([[0xc9,0,0]],'needs_nz_repair'),([[0x83,3]],'not_proven_redundant'),
                              ([[0x22,0,2,1]],'not_proven_redundant'),([[0x8f,0,0,0]],'not_proven_redundant'),
                              ([[0xe2,0x20],[0xa9,1],[0x83,3],[0xc2,0x20]],'not_proven_redundant')]:
            code=[[0xa3,2]]+middle+[[0xa3,2]];end=100+sum(map(len,code))
            op=operation(0,100,end,kind='binary');op['producer']=0;op['consumer']=stack(2,0)
            r,ins=routine(code,[op]);self.assertEqual(reloads(r,ins)[-1]['classification'],reason)
        for kind,extra in [('load',dict(volatile=True)),('load',dict(address=dict(kind='indirect'))),('call',{})]:
            op=operation(0,100,106,kind=kind,**extra)
            r,ins=routine([[0xa3,2],[0x83,4],[0xa3,2]],[op])
            self.assertEqual(reloads(r,ins)[-1]['classification'],'not_proven_redundant')
        op=operation(0,100,104,kind='binary');op['producer']=0;op['consumer']=stack(2,0)
        r,ins=routine([[0xa3,2],[0xa3,2]],[op],labels=[102])
        self.assertEqual(reloads(r,ins)[-1]['classification'],'not_proven_redundant')

    def test_frame_and_parameter_loads_are_new_policy_not_existing_temp_forwarding(self):
        for kind,expected in [('frame','frame_reload'),('parameter','parameter_reload')]:
            ops=[operation(0,100,104,kind='store'),operation(1,104,108,kind='load',width=2,volatile=False,address=dict(kind=kind,indexed=False,offset=2))]
            r,ins=routine([[0xa3,6],[0x83,2],[0xa3,2],[0x83,4]],ops)
            self.assertEqual(reloads(r,ins)[-1]['classification'],expected)

    def test_cfg_includes_dead_destinations_successor_live_ins_and_calls(self):
        r=dict(blocks=[dict(id=0,params=[],ops=[dict(index=0,uses=[0],definition=1)],terminator_uses=[1],successors=[1]),
                       dict(id=1,params=[2,3],ops=[dict(index=0,uses=[0,2],definition=4)],terminator_uses=[4],successors=[])])
        points=interference(r)
        self.assertTrue(any({0,1}<=live for _,live in points))
        self.assertTrue(any({0,2,3}<=live for _,live in points))
        self.assertTrue(any({0,2,4}<=live for _,live in points))
        self.assertFalse(any({1,2}<=live for _,live in points))

    def test_coalescing_distinguishes_pair_from_third_party_home_conflict(self):
        points=[(dict(point='closed_operation'),{0,2})]
        homes={0:stack(2),1:stack(4),2:stack(4)}
        m=move(stack(2,0),4,1)
        m['coalescing_attempts']=[dict(changed=0,onto=1,verifier_error='overlapping live stack temporaries'),dict(changed=1,onto=0,verifier_error=None)]
        c=coalescing(m,points,homes)
        self.assertEqual(c['classification'],'compatible_pair')
        self.assertEqual(c['directions'][0]['blocking_temps'],[2])
        bad=copy.deepcopy(m);bad['coalescing_attempts'][0]['verifier_error']=None
        with self.assertRaises(AssertionError):coalescing(bad,points,homes)


if __name__=='__main__':unittest.main()
