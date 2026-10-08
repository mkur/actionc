import unittest
from pathlib import Path

from exec_call_measure import commands
from exec_record_baseline import PROFILES


class MeasurementCommandsTests(unittest.TestCase):
    def test_all_profiles_use_their_authenticated_layout_and_actual_source(self):
        jobs = dict(commands(Path('/frozen'), Path('/candidate'), Path('/candidate/compiler')))
        self.assertEqual(jobs.keys(), PROFILES.keys())
        for name, command in jobs.items():
            self.assertEqual(command[0], '/candidate/compiler')
            self.assertEqual(command[-1], f'/frozen/profiles/{name}/kernel-program.act')
            self.assertEqual(command[command.index('--layout')+1], f'/candidate/{name}/probe.layout.json')
            self.assertEqual(command[command.index('-o')+1], '/candidate/host-image.json')
            self.assertEqual('--no-opt' in command, name == 'raw-guarded')
            self.assertEqual(command.count('--module-path'), 13)


if __name__ == '__main__':
    unittest.main()
