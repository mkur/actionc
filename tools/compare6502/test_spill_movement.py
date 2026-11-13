import json
from pathlib import Path
import tempfile
import unittest

from spill_movement import audit


def home(number):
    return dict(kind='spill', routine=1, id=number, offset=0)


class MovementAuditTests(unittest.TestCase):
    def capture(self, ambiguous=False):
        events = []

        def event(number, attempt, kind, **data):
            events.append(dict(id=number, attempt=attempt, kind=kind, data=data))

        event(0, 10, 'attempt-start', parent=0, stage='inlining')
        event(1, 11, 'attempt-start', parent=10, stage='core-with-helpers')
        event(2, 12, 'attempt-start', parent=11, stage='core-materialization')
        event(3, 13, 'attempt-start', parent=12, stage='rewrite-driver')
        key = dict(family='example', block=0, start=0)
        event(4, 13, 'core-layout-round', routine=1, generation=0,
              applied_count=1, applied=[dict(family='other')],
              candidates=[dict(key=key, end=1, outcome='overlap-rejected')],
              observations=[dict(evaluations=1, observation=dict(
                  candidate=key, predicate='address-query',
                  inputs=[dict(home=home(10), address=0x3002, bytes=1)]))])
        event(5, 12, 'home-remap', entries=[{'from':home(10), 'to':home(1)}])
        event(6, 11, 'snapshot', stage='attempt-return', program_identity='selected')
        event(7, 10, 'snapshot', stage='attempt-return', program_identity='selected')
        event(8, 21, 'attempt-start', parent=10, stage='core-with-helpers')
        event(9, 22, 'attempt-start', parent=21, stage='core-materialization')
        event(10, 21, 'snapshot', stage='attempt-return', program_identity='selected' if ambiguous else 'rejected')
        event(11, 12, 'core-layout-unattributed', observations=[dict(evaluations=1,
              observation=dict(predicate='address-query', inputs=[dict(home=home(10), address=0x3002, bytes=1)]))])
        frames = [dict(routine=1, slots=[], spills=[home(0), home(1)])]
        event(12, 30, 'snapshot', stage='attempt-input', routines=[dict(id=1,name='Example')], frames=frames)
        event(13, 30, 'snapshot', stage='proposed', frames=[dict(routine=1,slots=[],spills=[home(1)])])
        event(14, 30, 'core-allocation-movement', before_snapshot=12, proposed_snapshot=13,
              delta=dict(status='complete', omitted=0, non_frame_facts_preserved=True,
                         affected_routines=[1], changes=[
                  dict(home=home(0),before=dict(address=0x3000,bytes=1),after=None,expected_removal=True),
                  dict(home=home(1),before=dict(address=0x3001,bytes=1),after=dict(address=0x3000,bytes=1),expected_removal=False)]))
        event(15, 30, 'late-spill-pruning-result', reason='CoreAllocation', accepted=False)
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory/'manifest.json').write_text(json.dumps(dict(status='complete',core_layout_census=dict(status='complete'),work={},output_sizes=dict(load_file_bytes=1))))
            (directory/'events.jsonl').write_text(''.join(json.dumps(e)+'\n' for e in events))
            return audit(directory)

    def test_rejected_candidate_and_unscoped_epoch_do_not_acquire_acceptance(self):
        report = self.capture()
        self.assertEqual(report['selected_core']['core_attempts'], [12])
        counts = report['selected_core_input_evaluations']
        self.assertEqual(counts['example/overlap-rejected/address-query']['moved-survivor'], 1)
        self.assertEqual(counts['outside-driver/address-query']['unknown-unscoped-spill-epoch'], 1)
        self.assertEqual(report['examples'][0]['remap_events'], [5])
        self.assertEqual(report['examples'][0]['mapped_home'], home(1))

    def test_equivalent_trials_remain_ambiguous(self):
        report = self.capture(ambiguous=True)
        self.assertEqual(report['selected_core']['status'], 'unknown-or-ambiguous')
        self.assertEqual(report['selected_core_input_evaluations'], {})


if __name__ == '__main__':
    unittest.main()
