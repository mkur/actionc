import copy
import unittest

from exec_call_audit import audit_profile, csv_text, diagnostic, location_access, touches_home


def call_fixture(capture=False):
    # One padded word argument built with pushes, JSL, six-byte S cleanup,
    # optional word result capture, then RTL. ABI summary accesses on JSL
    # must not be counted as physical caller instructions reading arguments.
    code = bytes.fromhex('e2 20 a9 00 48 c2 20 a9 34 12 48 22 00 00 01 3b 18 69 03 00 1b')
    ranges = [(0,2,0),(2,4,0),(4,5,0),(5,7,1),(7,10,1),(10,11,1),(11,15,3),(15,16,3),(16,17,3),(17,20,3),(20,21,3)]
    effects = [dict(start=s,end=e,depth=d,source=[0,0],control='call' if s==11 else 'next',memory=[]) for s,e,d in ranges]
    effects[2]['memory'] = [dict(kind='stack',offset=0,bytes=1,access='write')]
    effects[5]['memory'] = [dict(kind='stack',offset=-1,bytes=2,access='write')]
    effects[6]['memory'] = [dict(kind='stack',offset=1,bytes=2,access='read'),dict(kind='unknown',bytes=None,access='may_write')]
    temps, physical_temps = [], []
    if capture:
        code += bytes.fromhex('85 a0')
        effects.append(dict(start=21,end=23,depth=0,source=[0,0],control='next',memory=[dict(kind='dp',offset=160,bytes=2,access='write')]))
        temps = [dict(id=10,width=2,pointer=False,producer=dict(kind='Call',block=0,index=0),uses=[dict(kind='Return',block=0,index=1,ordinal=0)],crossings=[],home=dict(kind='dp',offset=160,width=2))]
        physical_temps = [dict(id=10,size=2,home=dict(kind='direct_page',offset=160))]
    end = len(code)
    code += bytes.fromhex('6b')
    effects.append(dict(start=end,end=end+1,depth=0,source=[0,1],control='return',memory=[dict(kind='stack',offset=1,bytes=3,access='read')]))
    name = 'M_TEST_MAIN_0000'
    p = dict(id=0,name=name,address=0x10000,size=len(code),fixed_frame=0,spill_bytes=0,local_stack_peak=6,calls=[{}],temporaries=physical_temps)
    image = dict(format='actionc-65816-image',version=4,abi='action65816.native.v2',stack_checks=False,stack_overflow=0x48000,
        segments=[dict(address=0x10000,executable=True,bytes=list(code))],zero_fill=[],routines=[p])
    ir = dict(routines=[dict(id=0,blocks=[{}],spans=[dict(block=0,index=0,start=0,end=end,kind='Call/Direct'),dict(block=0,index=1,start=end,end=len(code),kind='Terminator/Return')])])
    call = dict(block=0,index=0,target_kind='direct',target_id=1,target_name='callee',outgoing_bytes=3,
        arguments=[dict(width=2,offset=0,value=dict(kind='constant',value=0x1234,width=2))],result=dict(id=10,width=2) if capture else None,
        declared_result_bytes=2 if capture else 0,live_across=[],cyclic=False,span=[0,end])
    typed = dict(schema=1,routines=[dict(id=0,name=name,frame=0,spill=0,local_peak=6,calls=[call],temps=temps,objects=[],instructions=effects,requests=[],helper=None)])
    placement = dict(routines=[],opaque_routines=[])
    return image,ir,typed,placement


class CallAuditTests(unittest.TestCase):
    def test_full_phase_partition_and_abi_summary_exclusion(self):
        summary,calls,_,_,_ = audit_profile(*call_fixture())
        self.assertEqual(summary['call_phases'],dict(pre_transfer_bytes=11,transfer_bytes=4,post_transfer_bytes=6,guard_bytes=0,non_guard_bytes=21))
        self.assertEqual(summary['static_nontransfer_access_bytes'],{'outgoing_stack_write_bytes':3})
        self.assertEqual(calls[0]['shape'],'pushes')
        self.assertEqual(summary['cohorts']['void_calls'],1)

    def test_result_capture_uses_complete_actual_home(self):
        summary,calls,_,_,captures = audit_profile(*call_fixture(True))
        self.assertEqual(summary['call_phases']['post_transfer_bytes'],8)
        self.assertEqual(calls[0]['result_capture_write_code_bytes'],2)
        self.assertEqual(summary['cohorts']['materialized_result_adjacent_return'],1)
        self.assertEqual(captures[0]['diagnostic'],'direct_page_home')
        self.assertEqual(summary['static_nontransfer_access_bytes']['dp_scratch_write_bytes'],2)

    def test_reserved_result_home_does_not_prove_a_capture(self):
        image,ir,typed,placement=call_fixture(True)
        image['segments'][0]['bytes'][21:23]=[]
        image['routines'][0]['size']-=2
        ir['routines'][0]['spans'][0]['end']-=2
        ir['routines'][0]['spans'][1].update(start=21,end=22)
        row=typed['routines'][0]
        row['calls'][0]['span'][1]-=2
        row['instructions'].pop(-2)
        row['instructions'][-1].update(start=21,end=22)
        summary,calls,_,_,captures=audit_profile(image,ir,typed,placement)
        self.assertEqual(captures[0]['home'],'dp')
        self.assertEqual(calls[0]['result_capture_write_code_bytes'],0)
        self.assertEqual(summary['cohorts']['result_values_with_capture_writes'],0)
        self.assertEqual(summary['cohorts']['adjacent_return_values_with_capture_writes'],0)

    def test_terminal_forward_has_its_own_transfer_and_no_call_frame(self):
        image,ir,typed,placement=call_fixture()
        image['segments'][0]['bytes']=list(bytes.fromhex('5c 00 00 01'))
        image['routines'][0].update(size=4,local_stack_peak=0,calls=[])
        ir['routines'][0]['spans']=[dict(block=0,index=0,start=0,end=4,kind='Call/Direct')]
        row=typed['routines'][0]
        row['local_peak']=0
        row['calls'][0].update(span=[0,4],arguments=[],outgoing_bytes=1)
        row['instructions']=[dict(start=0,end=4,depth=0,source=[0,0],control='forward',
            memory=[dict(kind='stack',offset=1,bytes=3,access='read')])]
        summary,calls,_,_,_=audit_profile(image,ir,typed,placement)
        self.assertEqual(summary['physical_transfers'],{'forward':1})
        self.assertEqual(summary['call_phases'],dict(pre_transfer_bytes=0,transfer_bytes=4,post_transfer_bytes=0,guard_bytes=0,non_guard_bytes=4))
        self.assertEqual(summary['static_nontransfer_access_bytes'],{})
        self.assertEqual(calls[0]['shape'],'terminal_forward')

    def test_missing_duplicate_and_forged_observations_are_rejected(self):
        for mutation in ('missing_effect','overlap','inside_operand','call_span','unknown_temp','wrong_home','missing_call','schema'):
            with self.subTest(mutation=mutation):
                image,ir,typed,placement=call_fixture(True)
                row=typed['routines'][0]
                if mutation=='missing_effect': row['instructions'].pop()
                elif mutation=='overlap': row['instructions'].append(copy.deepcopy(row['instructions'][0]))
                elif mutation=='inside_operand': row['instructions'][0]['end']=1
                elif mutation=='call_span': row['calls'][0]['span'][1]-=1
                elif mutation=='unknown_temp': row['calls'][0]['arguments'][0]['value']=dict(kind='temp',id=999)
                elif mutation=='wrong_home': row['temps'][0]['home']['offset']+=1
                elif mutation=='missing_call': row['calls']=[]
                else: typed['schema']=999
                with self.assertRaises(ValueError): audit_profile(image,ir,typed,placement)

    def test_stack_coordinates_follow_partial_argument_construction(self):
        effect=dict(kind='stack',offset=9,bytes=3,access='write')
        home=dict(kind='stack',offset=5,width=3)
        self.assertTrue(touches_home(effect,dict(depth=12),home,8))
        self.assertFalse(touches_home(effect,dict(depth=11),home,8))
        self.assertEqual(location_access(dict(kind='stack',offset=-1,bytes=2),dict(depth=10),dict(frame=8)),'outgoing_stack')
        self.assertEqual(location_access(dict(kind='dp',offset=0xbf,bytes=2),dict(depth=0),dict(frame=0)),'dp_other')

    def test_diagnostic_categories_preserve_materialization_and_call_crossings(self):
        t=dict(home=dict(kind='stack'),crossings=[[0,1]],width=4,producer=dict(kind='Call',block=0),uses=[])
        self.assertEqual(diagnostic(t),'lifetime_crosses_call')
        t['crossings']=[]
        self.assertEqual(diagnostic(t),'call_result')
        t['home']=None
        self.assertEqual(diagnostic(t),'no_materialized_home')

    def test_csv_preserves_quoted_names_and_host_line_endings(self):
        import csv
        import io
        text=csv_text([dict(routine='name,quoted',bytes=7)])
        for host_text in (text,text.replace('\n','\r\n')):
            self.assertEqual(list(csv.DictReader(io.StringIO(host_text,newline=''))),[dict(routine='name,quoted',bytes='7')])


if __name__ == '__main__':
    unittest.main()
