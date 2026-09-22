import copy
import json
from pathlib import Path
import tempfile
import unittest
from check_analysis_rewrite import check, digest, KNOWN_FAILURE


class FoundationEquality(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.before, self.after = [self.root/n for n in ('before', 'after')]
        for directory in (self.before, self.after):
            directory.mkdir()
            data = {'bytes': [0xA9, 1, 0], 'fixups': [{'offset': 1, 'target': 7}],
                    'map': [{'id': 1, 'offset': 2}]}
            (directory/'image.json').write_text(json.dumps(data))
            manifest = dict(schema=1, target='65816', cases=['loop', 'unlink'],
                            tools={n: {'sha256': n} for n in ('vbcc', 'vasm', 'vlink')},
                            artifacts=[dict(case='loop', mode='optimized', compiler='actionc',
                                            directory=str(directory), commands=[], image=str(directory/'image.json'),
                                            hashes={'image.json': digest(directory/'image.json')},
                                            routines=[{'fixed_frame': 8}], guard_ranges=[[0, 4]])])
            records = [dict(case='loop', mode='optimized', compiler='actionc', vector=0,
                            correct=True, cycles=735, dp_reads=68, stack_writes=32,
                            forwarded_word_loads=8, x_increment_updates=8),
                       dict(zip(('case', 'mode', 'compiler', 'vector'), KNOWN_FAILURE), correct=False)]
            report = dict(manifest=manifest, measurements=records,
                          control=[{'case': 'loop', 'sites': [{'pc': 1, 'target': 8, 'predicate': 0x90}]}])
            self.save(directory, report)
        self.baseline = dict(comparison_directory=str(self.before),
                             measurements={str(self.before/n): digest(self.before/n)
                                           for n in ('manifest.json', 'debug.json', 'release.json')},
                             qualification_record={}, snapshot_hashes={}, required_external_failures=[KNOWN_FAILURE])
        self.original = self.read()

    def read(self):
        return json.loads((self.after/'debug.json').read_text())

    def save(self, directory, report):
        (directory/'manifest.json').write_text(json.dumps(report['manifest']))
        for profile in ('debug', 'release'):
            (directory/(profile+'.json')).write_text(json.dumps(report))

    def test_identity(self):
        result = check(self.before, self.after, self.baseline)
        self.assertTrue(result['complete_control_and_observer_records_equal'])
        self.assertTrue(result['frozen_comparison_authenticated'])

    def test_records_and_control_negative_controls(self):
        for field in ('cycles', 'dp_reads', 'stack_writes', 'forwarded_word_loads', 'x_increment_updates', 'correct'):
            with self.subTest(field=field):
                report = copy.deepcopy(self.original)
                report['measurements'][0][field] += 1
                self.save(self.after, report)
                with self.assertRaises(AssertionError): check(self.before, self.after, self.baseline)
        for field in ('pc', 'target', 'predicate'):
            with self.subTest(field=field):
                report = copy.deepcopy(self.original)
                report['control'][0]['sites'][0][field] += 1
                self.save(self.after, report)
                with self.assertRaises(AssertionError): check(self.before, self.after, self.baseline)
        for field in ('control', 'new_observer'):
            report = copy.deepcopy(self.original)
            report[field] = []
            self.save(self.after, report)
            with self.assertRaises(AssertionError): check(self.before, self.after, self.baseline)

    def test_artifact_bytes_fixups_maps_and_storage(self):
        original = (self.after/'image.json').read_text()
        for field in ('bytes', 'fixups', 'map'):
            with self.subTest(field=field):
                image = json.loads(original)
                image[field] = []
                (self.after/'image.json').write_text(json.dumps(image))
                report = copy.deepcopy(self.original)
                # Even a correctly rehashed mutation must fail equality.
                report['manifest']['artifacts'][0]['hashes']['image.json'] = digest(self.after/'image.json')
                self.save(self.after, report)
                with self.assertRaises(AssertionError): check(self.before, self.after, self.baseline)
        (self.after/'image.json').write_text(original)
        for field, value in (('guard_ranges', []), ('routines', [{'fixed_frame': 6}])):
            report = copy.deepcopy(self.original)
            report['manifest']['artifacts'][0][field] = value
            self.save(self.after, report)
            with self.assertRaises(AssertionError): check(self.before, self.after, self.baseline)

    def test_frozen_hashes_and_wrong_before(self):
        bad = copy.deepcopy(self.baseline)
        bad['measurements'][str(self.before/'debug.json')] = 'changed'
        with self.assertRaises(AssertionError): check(self.before, self.after, bad)
        with self.assertRaises(AssertionError): check(self.after, self.after, self.baseline)
        for field in ('qualification_record', 'snapshot_hashes'):
            bad = copy.deepcopy(self.baseline)
            bad[field] = {str(self.before/'image.json'): 'changed'}
            with self.assertRaises(AssertionError): check(self.before, self.after, bad)

    def test_external_failure_is_explicit_even_when_both_sides_agree(self):
        for failures in ([], [KNOWN_FAILURE, ['loop', 'optimized', 'actionc', 0]]):
            for directory in (self.before, self.after):
                report = json.loads((directory/'debug.json').read_text())
                for r in report['measurements']:
                    r['correct'] = [r[k] for k in ('case', 'mode', 'compiler', 'vector')] not in failures
                self.save(directory, report)
            for p in self.baseline['measurements']:
                self.baseline['measurements'][p] = digest(Path(p))
            with self.assertRaises(AssertionError): check(self.before, self.after, self.baseline)
