import copy
import unittest

from exec_call_flow import addressed_temps, capture_rows, comparison, home_read_bytes, investigate, private_source_reason


def private_fixture():
    address=dict(kind='immutable_parameter',id=0,displacement=0,indexed=False)
    producer=dict(kind='Load',dest=0,width=2,address=address,volatile=False)
    routine=dict(parameters=[dict(id=0,width=2,object=None)],objects=[],blocks=[dict(id=0,ops=[producer])])
    return producer,routine


class CallFlowTests(unittest.TestCase):
    def test_zero_test_normalizes_operand_side_without_guessing(self):
        op=dict(kind='Compare',width=1,signed=False,operation='Eq',left=dict(kind='temp',id=4),right=dict(kind='constant',value=0))
        self.assertTrue(comparison(op,4)['scalar_zero_test'])
        op['left'],op['right']=op['right'],op['left']
        self.assertEqual(comparison(op,4)['side'],'right')
        op['operation']='Lt'
        self.assertFalse(comparison(op,4)['scalar_zero_test'])
        op.update(operation='Eq',width=4)
        self.assertFalse(comparison(op,4)['scalar_zero_test'])
        op['left']=dict(kind='temp',id=4)
        with self.assertRaises(ValueError):comparison(op,4)

    def test_nonconstant_zero_is_not_a_literal_zero_test(self):
        op=dict(kind='Compare',width=2,signed=False,operation='Ne',left=dict(kind='temp',id=4),right=dict(kind='temp',id=0))
        self.assertFalse(comparison(op,4)['scalar_zero_test'])

    def test_private_read_requires_complete_unexposed_storage(self):
        p,r=private_fixture()
        self.assertEqual(private_source_reason(p,r,[]),'bounded_private_candidate')
        for mutation,expected in [('partial','partial_home'),('offset','noncanonical_view'),('volatile','nonprivate_or_volatile'),('exposed','addressed_or_mixed_views'),('mixed','addressed_or_mixed_views'),('other_offset','addressed_or_mixed_views')]:
            with self.subTest(mutation=mutation):
                p,r=private_fixture()
                if mutation=='partial':p['width']=1
                elif mutation=='offset':p['address']['displacement']=1
                elif mutation=='volatile':p['volatile']=True
                elif mutation=='exposed':r['blocks'][0]['ops'].append(dict(kind='AddressOf',width=2,address=copy.deepcopy(p['address'])))
                elif mutation=='mixed':r['blocks'][0]['ops'].append(dict(kind='Load',width=1,volatile=False,address=copy.deepcopy(p['address'])))
                else:r['blocks'][0]['ops'].append(dict(kind='Load',width=2,volatile=False,address=dict(p['address'],displacement=1)))
                self.assertEqual(private_source_reason(p,r,[]),expected)

    def test_private_borrowing_screen_keeps_ordering_barriers(self):
        p,r=private_fixture()
        for gap in ([dict(kind='Call')],[dict(kind='Store')],[dict(kind='Copy')],[dict(kind='Load',volatile=True)]):
            self.assertEqual(private_source_reason(p,r,gap),'intervening_barrier')
        self.assertEqual(private_source_reason(p,r,[dict(kind='Cast')]),'bounded_private_candidate')

    def test_parameter_frame_alias_is_not_treated_as_an_independent_local(self):
        p,r=private_fixture()
        p['address'].update(kind='private_frame',id=7)
        r['objects']=[dict(id=7,width=2,addressable=False)]
        r['parameters'][0]['object']=7
        self.assertEqual(private_source_reason(p,r,[]),'parameter_frame_alias')

    def test_address_use_is_distinguished_from_stored_value(self):
        self.assertEqual(addressed_temps(dict(base_value=dict(kind='temp',id=7),index=dict(value=dict(kind='temp',id=8)))),[7,8])
        self.assertEqual(addressed_temps(dict(base_value=dict(kind='constant',value=0),index=None)),[])

    def test_home_read_bytes_bind_source_depth_and_exclude_call_summaries(self):
        e=dict(start=10,end=12,source=[0,1],depth=12,control='next',memory=[dict(kind='stack',offset=8,bytes=2,access='read')])
        r=dict(frame=8,instructions=[e,dict(e,start=12,end=16,control='call'),dict(e,start=16,end=18,source=[0,2])])
        t=dict(home=dict(kind='stack',offset=4,width=2))
        self.assertEqual(home_read_bytes(r,t,(0,1)),2)
        e['memory'][0]['offset']+=1
        self.assertEqual(home_read_bytes(r,t,(0,1)),0)

    def test_actual_capture_parse_and_typed_join_accept_lf_and_crlf(self):
        name='main,quoted'
        result=dict(id=0,width=1,producer=dict(kind='Call',block=0,index=0),uses=[dict(kind='Compare',block=0,index=1)])
        condition=dict(id=1,producer=dict(kind='Compare',block=0,index=1),uses=[dict(kind='Branch',block=0,index=2)])
        typed=dict(routines=[dict(id=0,name=name,temps=[result,condition],calls=[dict(block=0,index=0)])])
        op=dict(kind='Compare',dest=1,width=1,signed=False,operation='Eq',left=dict(kind='temp',id=0),right=dict(kind='constant',value=0))
        flow=dict(schema=1,routines=[dict(id=0,name=name,blocks=[dict(id=0,ops=[dict(kind='Call'),op])])])
        text='profile,routine,temp,capture_write_code_bytes\noptimized-release,"main,quoted",0,2\n'
        for host in (text,text.replace('\n','\r\n')):
            report,results,arguments=investigate(typed,flow,capture_rows(host.encode()))
            self.assertEqual(report['counts']['scalar_zero_test_branch_results'],1)
            self.assertEqual(report['counts']['adjacent_result_capture_bytes'],2)
            self.assertEqual(results[0]['routine'],name)
            self.assertEqual(arguments,[])


if __name__=='__main__':
    unittest.main()
