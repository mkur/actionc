import copy
import tempfile
from pathlib import Path
import unittest

from exec_record_baseline import guard_ranges, inventory, verify_inputs, save, digest, require_projection
from exec_record_vectors import flow_cases


def fixture():
    image = dict(format='actionc-65816-image', version=4, abi='action65816.native.v2',
        stack_checks=False, stack_overflow=0x48000,
        segments=[dict(address=0x10000, executable=True, bytes=[0x6b]),
                  dict(address=0x120000, executable=False, bytes=[1, 2, 3])],
        zero_fill=[dict(size=7)], routines=[dict(id=1, name='M_TEST_MAIN_0000', address=0x10000,
        size=1, fixed_frame=0, spill_bytes=0, local_stack_peak=0, calls=[])])
    logical = dict(routines=[dict(id=1, blocks=[{}], spans=[dict(start=0, end=1, kind='Terminator/Return')])])
    return image, logical


class Accounting(unittest.TestCase):
    def test_exact_coverage_and_rejected_mutations(self):
        image, logical = fixture()
        summary, rows = inventory(image, logical)
        self.assertEqual((summary['code_bytes'], summary['initialized_data_bytes'], summary['zero_fill_bytes']), (1, 3, 7))
        self.assertEqual(summary['families'], {'Terminator': 1, 'Prologue/helpers/unspanned': 0})
        for mutation in ('overlap', 'outside', 'missing', 'opcode', 'version', 'segment'):
            with self.subTest(mutation=mutation):
                img, ir = copy.deepcopy(image), copy.deepcopy(logical)
                if mutation == 'overlap': ir['routines'][0]['spans'] *= 2
                elif mutation == 'outside': ir['routines'][0]['spans'][0]['end'] = 2
                elif mutation == 'missing': ir['routines'] = []
                elif mutation == 'opcode': img['segments'][0]['bytes'] = [0xff]
                elif mutation == 'version': img['version'] = 5
                else: img['routines'][0]['size'] = 2
                with self.assertRaises(ValueError): inventory(img, ir)

    def test_native_v2_guards_require_complete_instruction_bounded_encoding(self):
        image, _ = fixture()
        guard = bytes.fromhex('3b aa c5 c6 90 04 f0 02 80 0a 38 e9 18 00 90 04 c5 c4 b0 07 a9 18 00 5c 00 80 04')
        image['stack_checks'] = True
        image['segments'][0]['bytes'] = list(guard + b'\x6b')
        self.assertEqual(guard_ranges(image), [[0x10000, 0x1001b]])
        image['segments'][0]['bytes'][21] = 0x19
        with self.assertRaises(ValueError): guard_ranges(image)
        image['segments'][0]['bytes'] = list(guard)
        image['stack_checks'] = False
        with self.assertRaises(ValueError): guard_ranges(image)

    def test_platform_additions_preserve_exact_compiler_projection(self):
        compiler, _ = fixture()
        compiler['data'] = []
        packaged = copy.deepcopy(compiler)
        packaged['segments'].append(dict(address=0x3f1000, executable=True, bytes=[0xdb]))
        packaged['data'].append(dict(name='platform_metadata'))
        require_projection(packaged, compiler)
        packaged['segments'][0]['bytes'] = [0xea]
        with self.assertRaises(ValueError): require_projection(packaged, compiler)
        packaged = copy.deepcopy(compiler)
        packaged['routines'][0]['fixed_frame'] = 2
        with self.assertRaises(ValueError): require_projection(packaged, compiler)

    def test_independent_record_oracle_handles_wraps_and_preserves_alignment_padding(self):
        case = flow_cases()
        self.assertEqual(len(case['vectors']), 12)
        one = next(v for v in case['vectors'] if v['count'] == 1 and v['choose'] == 1)
        self.assertEqual(one['result'], 65532)
        two = next(v for v in case['vectors'] if v['count'] == 2 and v['choose'] == 0)
        self.assertEqual(two['result'], 360)
        for vector in case['vectors']:
            for row in vector['after']:
                self.assertEqual(row['bytes'][1], 0xa5)
                self.assertEqual(len(row['bytes']), 14)

    def test_input_mutations_cannot_reuse_a_frozen_baseline(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            paths = ('compiler/source', 'exec/source', 'exec/build/external', 'exec/build/of816-upstream/source')
            for name in paths:
                path = base / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b'original')
            pin = base / 'exec/toolchain/actionc.json'
            pin.parent.mkdir()
            save(pin, {'revision': 'baseline'})
            save(base / 'inputs.json', dict(compiler=dict(files={'source': digest(base / paths[0])}),
                exec=dict(files={'source': digest(base / paths[1]), 'toolchain/actionc.json': 'overridden'}),
                compiler_pin_override=dict(baseline={'revision': 'baseline'}),
                external_inputs={'external': digest(base / paths[2])},
                of816=dict(files={'source': digest(base / paths[3])})))
            verify_inputs(base)
            for name in (*paths, 'exec/toolchain/actionc.json'):
                with self.subTest(path=name):
                    path = base / name
                    before = path.read_bytes()
                    path.write_bytes(b'{}' if path == pin else b'changed')
                    with self.assertRaises(ValueError): verify_inputs(base)
                    path.write_bytes(before)


if __name__ == '__main__': unittest.main()
