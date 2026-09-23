import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


@unittest.skipUnless(sys.platform in ('darwin', 'linux'), 'wait4 accounting')
class MeasurementControls(unittest.TestCase):
    def test_real_child_accounting_rejects_wrong_images_and_cardinality(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            writer = root / 'writer.py'
            writer.write_text('from pathlib import Path\nimport sys\nPath(sys.argv[2]).write_bytes(b"image")\n')
            manifest = root / 'manifest.json'
            output = root / 'report.json'
            artifacts = [dict(compiler='actionc', case=str(i), mode='raw',
                commands=[[sys.executable, str(writer), '-o', str(root / 'image')]],
                hashes={'image.json': hashlib.sha256(b'image').hexdigest()}) for i in range(2)]
            manifest.write_text(json.dumps(dict(artifacts=artifacts)))
            command = [sys.executable, '-B', str(Path(__file__).with_name('measure_host.py')),
                '--before', sys.executable, '--after', sys.executable, '--manifest', str(manifest),
                '--output', str(output), '--rounds', '3', '--expected-builds', '2']
            result = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            report = json.loads(output.read_text())
            self.assertEqual(len(report['samples']), 12)
            self.assertTrue(all(s['peak_rss_bytes'] > 0 for s in report['samples']))
            output.unlink()
            for mutation in ('hash', 'count'):
                with self.subTest(mutation=mutation):
                    if mutation == 'hash':
                        artifacts[0]['hashes']['image.json'] = 'bad'
                    else:
                        artifacts.pop()
                    manifest.write_text(json.dumps(dict(artifacts=artifacts)))
                    self.assertNotEqual(subprocess.run(command, capture_output=True).returncode, 0)
                    self.assertFalse(output.exists())


if __name__ == '__main__':
    unittest.main()
