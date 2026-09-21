import copy
import hashlib
from pathlib import Path
import tempfile
import unittest
from check_state_tracker import equal_records, equal_artifact


class Equality(unittest.TestCase):
    def test_complete_records_including_unknown_metrics_and_pc_maps(self):
        old = {('loop', 'raw', 'actionc', 0): {'cycles': 9, 'future_metric': 3, 'sites': {'32': 4}}}
        equal_records(old, copy.deepcopy(old))
        for changed in ({}, {**old, ('extra',): {}}, {next(iter(old)): {'cycles': 8}},
                        {next(iter(old)): {**next(iter(old.values())), 'future_metric': 4}},
                        {next(iter(old)): {**next(iter(old.values())), 'sites': {'33': 4}}}):
            with self.assertRaises(AssertionError):
                equal_records(old, changed)

    def test_artifact_bytes_contracts_hashes_and_exact_header(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifacts = []
            for name in ('old', 'new'):
                path = root / name
                path.mkdir()
                data = f'Source: "{path}/code.asm"\nlda #1\n'.encode()
                (path / 'code.lst').write_bytes(data)
                artifacts.append(dict(case='loop', compiler='vbcc', directory=str(path), commands=[],
                                      routines=[{'address': 123, 'size': 4}],
                                      hashes={'code.lst': hashlib.sha256(data).hexdigest()}))
            old, new = artifacts
            equal_artifact(old, new)
            for field, value in [('routines', []), ('hashes', {}), ('compiler', 'actionc')]:
                with self.assertRaises(AssertionError):
                    equal_artifact(old, {**new, field: value})
            (Path(new['directory']) / 'code.lst').write_bytes(b'corrupted')
            with self.assertRaises(AssertionError):
                equal_artifact(old, new)
            new['hashes']['code.lst'] = hashlib.sha256(b'corrupted').hexdigest()
            with self.assertRaises(AssertionError):
                equal_artifact(old, new)
