import copy
import tempfile
import unittest
from pathlib import Path

from exec_call_candidate import CANDIDATE_FILES, authenticate, resource_deltas
from exec_record_baseline import PROFILES, digest, save


class CandidateTests(unittest.TestCase):
    def test_changed_code_is_allowed_but_resource_growth_or_identity_drift_is_not(self):
        before = [dict(routine='r', code_bytes=100, frame=8, spill=6, local_peak=15)]
        after = [dict(routine='r', code_bytes=90, frame=6, spill=4, local_peak=13)]
        self.assertEqual(resource_deltas(before, after)[0]['code_bytes'], -10)
        for field in ('routine', 'frame', 'spill', 'local_peak'):
            bad = copy.deepcopy(after)
            bad[0][field] = 'other' if field == 'routine' else before[0][field]+1
            with self.subTest(field=field), self.assertRaises(ValueError):
                resource_deltas(before, bad)
        with self.assertRaises(ValueError):
            resource_deltas(before, after*2)

    def test_every_profile_and_artifact_is_authenticated(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            facts = dict(schema=1, kind='call-flow-candidate', profiles={})
            for profile in PROFILES:
                (root/profile).mkdir()
                for name in CANDIDATE_FILES:
                    (root/profile/name).write_text('{}\n')
                facts['profiles'][profile] = dict(artifacts={n: digest(root/profile/n) for n in CANDIDATE_FILES})
            save(root/'provenance.json', facts)
            authenticate(root)
            (root/'optimized-release/probe.flow.json').write_text('{"forged":true}\n')
            with self.assertRaises(ValueError):
                authenticate(root)
            facts['profiles'].pop('raw-guarded')
            save(root/'provenance.json', facts)
            with self.assertRaises(ValueError):
                authenticate(root)


if __name__ == '__main__':
    unittest.main()
