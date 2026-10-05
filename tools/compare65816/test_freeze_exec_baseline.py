import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from exec_record_baseline import verify_inputs


def git(root, *args):
    return subprocess.run(['git', '-C', str(root), *args], check=True, capture_output=True)


def repository(path, files):
    path.mkdir(parents=True)
    git(path, 'init')
    git(path, 'config', 'user.email', 'baseline@example.invalid')
    git(path, 'config', 'user.name', 'Baseline test')
    for name, value in files.items():
        target = path / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(value)
    git(path, 'add', '.')
    git(path, 'commit', '-m', 'fixture')


class FrozenSources(unittest.TestCase):
    def test_real_worktrees_preserve_dirty_sources_and_copy_symlinked_sdks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            compiler, execroot, output = root / 'compiler-live', root / 'exec-live', root / 'snapshot'
            repository(compiler, {'source': 'committed compiler'})
            repository(execroot, {'examples/source.act': 'committed Exec', 'examples/deleted.act': 'deleted',
                'toolchain/actionc.json': json.dumps({'revision': 'old', 'abi': 'action65816.native.v2'})})
            (compiler / 'source').write_text('unrelated live change')
            (execroot / 'examples/source.act').write_text('dirty source\r\n')
            (execroot / 'examples/deleted.act').unlink()
            (execroot / 'examples/untracked.act').write_text('new source')
            for folder in ('firmware', 'shell-paced-bridge', 'altirra-irq-bridge', 'altirra-sio-multi'):
                directory = execroot / 'build' / folder
                directory.mkdir(parents=True)
                if folder == 'firmware':
                    (directory / 'altirraos-816.rom').write_bytes(b'ROM')
                else:
                    (directory / 'AltirraBridgeServer').write_bytes(b'bridge')
                    sdk = root / (folder + '-sdk') / 'python'
                    sdk.mkdir(parents=True)
                    (sdk / 'altirra_bridge.py').write_text('SDK input')
                    (directory / 'sdk').symlink_to(sdk.parent, target_is_directory=True)
            upstream = execroot / 'build/of816-upstream'
            repository(upstream, {'source': 'OF816 input'})
            external = execroot / 'build/altirra-irq-fix'
            for directory in ('src/compat', 'src/h'): (external / directory).mkdir(parents=True)
            for name in ('src/ATIO/source/diskfssdx2.cpp', 'src/ATIO/source/diskfssdx2util.cpp',
                         'build/latency/src/ATIO/libATIO.a', 'build/latency/src/ATCore/libATCore.a',
                         'build/latency/src/system/libsystem.a'):
                path = external / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b'external')
            result = subprocess.run([sys.executable, '-B', str(Path(__file__).with_name('freeze_exec_baseline.py')),
                '--compiler-checkout', str(compiler), '--exec-checkout', str(execroot), '--base', str(output)],
                capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            verify_inputs(output)
            self.assertEqual((output / 'compiler/source').read_text(), 'committed compiler')
            self.assertEqual((output / 'exec/examples/source.act').read_bytes(), b'dirty source\r\n')
            self.assertFalse((output / 'exec/examples/deleted.act').exists())
            self.assertEqual((output / 'exec/examples/untracked.act').read_text(), 'new source')
            self.assertEqual((compiler / 'source').read_text(), 'unrelated live change')
            self.assertEqual((execroot / 'examples/source.act').read_bytes(), b'dirty source\r\n')
            self.assertEqual((output / 'exec/build/altirra-irq-bridge/sdk/python/altirra_bridge.py').read_text(), 'SDK input')
            repeated = subprocess.run([sys.executable, '-B', str(Path(__file__).with_name('freeze_exec_baseline.py')),
                '--compiler-checkout', str(compiler), '--exec-checkout', str(execroot), '--base', str(output)],
                capture_output=True, text=True)
            self.assertNotEqual(repeated.returncode, 0)


if __name__ == '__main__': unittest.main()
