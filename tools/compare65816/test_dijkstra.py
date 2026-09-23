"""Fail-closed source adaptation and artifact-authentication controls."""
import json
from pathlib import Path
import tempfile
import unittest
from dijkstra import FIXTURE, sources, replace_once
from build import digest
from run_dijkstra import verify


class DijkstraControls(unittest.TestCase):
    def test_actual_source_adaptation_accepts_crlf(self):
        normal = sources()
        self.assertEqual(sources(lambda name: (FIXTURE/name).read_text().replace('\n','\r\n')), normal)
        self.assertIn('j=50', normal['kernel.inc'])
        self.assertNotIn('MOD', normal['kernel.inc'])
        # Search and queue routines are copied byte-for-byte after normalization.
        self.assertEqual(normal['kernel.inc'].split('PROC Benchmark()')[0],
                         (FIXTURE/'kernel.inc').read_text().split('PROC Benchmark()')[0])

    def test_changed_upstream_is_rejected(self):
        with self.assertRaises(AssertionError):
            sources(lambda name: (FIXTURE/name).read_text()+(' ' if name=='dijkstra.c' else ''))
        for text in ('absent','x x'):
            with self.assertRaises(AssertionError):
                replace_once(text,'x','y')

    def test_artifact_mutation_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            p=Path(directory)/'code.bin';p.write_bytes(b'\x6b')
            manifest=dict(inputs={},generated={},tools={},artifacts=[dict(hashes={str(p):digest(p)})])
            verify(manifest)
            p.write_bytes(b'\xdb')
            with self.assertRaises(ValueError):
                verify(manifest)


if __name__=='__main__':
    unittest.main()
