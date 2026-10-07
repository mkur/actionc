import copy
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from exec_record_qualification import acceptance, checked_summaries, hosted_disposition, profile_comparison, measured_boot
from exec_record_hosted import measured_emulator


class QualificationChecks(unittest.TestCase):
    def inputs(self):
        targets = dict(release_compiler_code_bytes_max=422700, representative_release_code_bytes_max=9235,
                       native_release_private_accesses_max=20565, native_release_cycles_max=78117,
                       optimized_native_stack_peak_max=42)
        release = dict(after_bytes=422700, representative_after=9235, benefited_subsystems=['A', 'B', 'C'],
                       benefits=dict(loop=['Loop'], call=['Caller']),
                       native=dict(after=dict(cycles=78117, private=20565, peak=42)))
        profiles = {'optimized-release': release, 'optimized-guarded': copy.deepcopy(release)}
        hosted = {case+'-optimized-release': dict(status='pass') for case in ('demo', 'of816')}
        return profiles, hosted, targets

    def test_final_limits_accept_exact_boundaries(self):
        profiles, hosted, targets = self.inputs()
        self.assertTrue(acceptance(profiles, hosted, targets)['passed'])

    def test_each_missed_numerical_gate_remains_a_failure(self):
        for field, key in (('after_bytes', 'release_compiler_code_bytes_max'),
                           ('representative_after', 'representative_release_code_bytes_max')):
            profiles, hosted, targets = self.inputs()
            profiles['optimized-release'][field] += 1
            self.assertEqual(acceptance(profiles, hosted, targets)['failed'], [key])
        for field, key in (('private', 'native_release_private_accesses_max'),
                           ('cycles', 'native_release_cycles_max'), ('peak', 'optimized_native_peak')):
            profiles, hosted, targets = self.inputs()
            profiles['optimized-release']['native']['after'][field] += 1
            self.assertEqual(acceptance(profiles, hosted, targets)['failed'], [key])

    def test_guarded_success_never_qualifies_an_unchecked_package(self):
        profiles, hosted, targets = self.inputs()
        hosted = {key.replace('optimized-release', 'optimized-guarded'): row for key, row in hosted.items()}
        self.assertEqual(acceptance(profiles, hosted, targets)['failed'], ['hosted_release'])
        for status in ('unsupported', 'fail', 'running'):
            profiles, hosted, targets = self.inputs()
            hosted['demo-optimized-release']['status'] = status
            self.assertIn('hosted_release', acceptance(profiles, hosted, targets)['failed'])

    def test_subsystems_loops_and_calls_are_separate_obligations(self):
        for kind in ('loop', 'call'):
            profiles, hosted, targets = self.inputs()
            profiles['optimized-release']['benefits'][kind] = []
            self.assertIn(kind+'_benefits', acceptance(profiles, hosted, targets)['failed'])
        profiles, hosted, targets = self.inputs()
        profiles['optimized-release']['benefited_subsystems'].pop()
        self.assertIn('subsystems', acceptance(profiles, hosted, targets)['failed'])

    def test_known_fixture_failures_keep_their_scope_and_owner(self):
        failures = [('lists', 'opt', 'Resident image payload in bank zero', None),
                    ('ports', 'raw', 'Native interrupt/COP vectors not restored', None),
                    ('dos-streams', 'opt', 'compiler failed', 'Unable to load console-storage-action.inc')]
        for case, mode, message, diagnostic in failures:
            record = dict(status='fail', error=message, diagnostic=diagnostic)
            self.assertEqual(hosted_disposition(case, mode, record, dict(status='fail'))['status'], 'pre-existing-failure')
            self.assertEqual(hosted_disposition(case, mode, record, dict(status='pass'))['status'], 'new-failure')
        record = dict(status='fail', error='Different compiler or platform failure')
        self.assertEqual(hosted_disposition('lists', 'raw', record, dict(status='fail'))['status'], 'unexplained-failure')

    def test_unexplained_hosted_failures_block_acceptance(self):
        for status in ('new-failure', 'unexplained-failure'):
            profiles, hosted, targets = self.inputs()
            hosted['ports-raw-guarded'] = dict(status='fail', disposition=dict(status=status))
            self.assertIn('hosted_regressions', acceptance(profiles, hosted, targets)['failed'])

    def test_resource_growth_and_changed_data_are_rejected(self):
        row = dict(name='M_A_F_1234', size=100, fixed_frame=8, spill_bytes=6, local_stack_peak=12)
        image = dict(abi='native.v2', task_headroom=26, irq_headroom=13,
                     data=[dict(id=0, address=0x10000, size=3)], routines=[row])
        placement = dict(routines=[dict(name=row['name'], loop_homes=1, call_segments=1)])
        with patch('exec_record_qualification.REPRESENTATIVES', ('A.F',)):
            after = copy.deepcopy(image); after['routines'][0]['size'] -= 1
            self.assertEqual(profile_comparison(image, after, placement)[0]['after_bytes'], 99)
            for field in ('fixed_frame', 'spill_bytes', 'local_stack_peak'):
                bad = copy.deepcopy(after); bad['routines'][0][field] += 1
                with self.subTest(field=field), self.assertRaises(ValueError):
                    profile_comparison(image, bad, placement)
            bad = copy.deepcopy(after); bad['data'][0]['address'] += 1
            with self.assertRaises(ValueError): profile_comparison(image, bad, placement)

    def test_actual_log_parsing_handles_lf_and_crlf_and_rejects_failures(self):
        source = 'running 2 tests\ntest result: ok. 2 passed; 0 failed; 1 ignored;\n'
        for text in (source, source.replace('\n', '\r\n')):
            self.assertEqual(checked_summaries(text), dict(targets=1, passed=2, ignored=1))
            with self.assertRaises(ValueError): checked_summaries(text+'test result: FAILED.\n')

    def test_boot_pin_projection_binds_candidate_and_preserves_frozen_paths(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root/'toolchain').mkdir()
            frozen = dict(revision='baseline', abi='native.v2', target='wdc-65816-native')
            original_text = json.dumps(frozen)+'\n'
            (root/'toolchain/actionc.json').write_text(original_text)
            toolchain = dict(revision='candidate', binary_sha256='binary', abi_sha256='abi', compiler_contract=frozen)
            program = dict(build={field: toolchain[field] for field in ('revision','binary_sha256','abi_sha256')})
            module = SimpleNamespace(ROOT=root, read_build=lambda path: program)

            def boot(output, exec_build, upstream):
                pin = json.loads((module.ROOT/'toolchain/actionc.json').read_text())
                self.assertEqual(pin, dict(frozen, revision='candidate'))
                self.assertEqual(module.ROOT/'build/firmware/rom', root/'build/firmware/rom')
                self.assertEqual((root/'platform/boot.s').relative_to(module.ROOT), Path('platform/boot.s'))
                return dict(format='boot')

            record = measured_boot(module, boot, toolchain, root/'output', root/'exec', root/'upstream')
            self.assertEqual(record['measured_compiler']['binary_sha256'], 'binary')
            self.assertEqual((root/'toolchain/actionc.json').read_text(), original_text)
            self.assertIs(module.ROOT, root)
            for field in ('revision','binary_sha256','abi_sha256'):
                wrong = copy.deepcopy(program); wrong['build'][field] = 'wrong'
                module.read_build = lambda path: wrong
                with self.assertRaises(ValueError):
                    measured_boot(module, boot, toolchain, root/'output', root/'exec', root/'upstream')
            module.read_build = lambda path: program
            def fail(*args): raise ValueError('boot failure')
            with self.assertRaises(ValueError):
                measured_boot(module, fail, toolchain, root/'output', root/'exec', root/'upstream')
            self.assertIs(module.ROOT, root)

    def test_actual_machine_check_precedes_boot_and_closes_on_rejection(self):
        from contextlib import contextmanager
        events = []
        @contextmanager
        def emulator(directory, rom, output, *, pin):
            events.append(('open', directory, rom, output, pin))
            try: yield 'bridge'
            finally: events.append('close')
        def verify(bridge, rom, pin): events.append(('verify',bridge,rom,pin))
        with measured_emulator(emulator, verify)('distribution','rom','output',pin='pin') as bridge:
            events.append(('boot',bridge))
        self.assertEqual(events, [('open','distribution','rom','output','pin'),
                                 ('verify','bridge','rom','pin'), ('boot','bridge'), 'close'])
        events.clear()
        def reject(*args): raise ValueError('Wrong mapped ROM')
        with self.assertRaises(ValueError):
            with measured_emulator(emulator, reject)('distribution','rom','output',pin='pin'):
                self.fail('An unverified machine must never run the boot fixture')
        self.assertEqual(events[-1], 'close')


if __name__ == '__main__':
    unittest.main()
