"""Reject stale input provenance through the real source-discovery path."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('native_qualification', Path(__file__).with_name('qualify.py'))
qualify = importlib.util.module_from_spec(spec)
spec.loader.exec_module(qualify)


class StableInputs(unittest.TestCase):
    def test_source_fixture_addition_deletion_and_content_changes_are_rejected(self):
        for mutation in ('source', 'fixture', 'addition', 'deletion', 'tool'):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                here = root / 'tools/native65816-runtime-tests'
                (root / 'src').mkdir(parents=True)
                (here / 'tests/fixtures').mkdir(parents=True)
                source = root / 'src/emit.rs'
                source.write_text('source\n')
                fixture = here / 'tests/fixtures/example.act'
                fixture.write_text('PROC Main()\nRETURN\n')
                script = here / 'qualify.py'
                script.write_text('qualifier\n')
                with patch.object(qualify, 'ROOT', root), patch.object(qualify, 'HERE', here), patch.object(qualify, '__file__', str(script)):
                    before = qualify.input_hashes()
                    qualify.require_stable_inputs(before)
                    if mutation == 'addition':
                        (root / 'src/new.rs').write_text('new source\n')
                    elif mutation == 'deletion':
                        source.unlink()
                    else:
                        target = dict(source=source, fixture=fixture, tool=script)[mutation]
                        target.write_text(target.read_text() + 'changed\n')
                    with self.assertRaisesRegex(RuntimeError, 'inputs changed during execution'):
                        qualify.require_stable_inputs(before)


if __name__ == '__main__':
    unittest.main()
