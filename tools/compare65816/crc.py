#!/usr/bin/env python3
"""Build the c-bench-64 CRC kernels for native 65816 speed comparison."""
import argparse
import importlib.util
import json
from pathlib import Path
import re
import sys

sys.dont_write_bytecode = True
from build import ROOT, digest, image_guard_ranges, run
from calypsi import LINKER_RULES

FIXTURES = ROOT / 'tools/native65816-runtime-tests/tests/fixtures/crc_bench'
DEFAULT_SUITE = Path.home() / 'atari/c-bench-64'


def extract(source, width):
    text = source.replace('\r\n', '\n')
    if width == 8:
        core = 'unsigned char CRC8' + text.split('#else\nstatic unsigned char CRC8', 1)[1].split('\n#endif', 1)[0]
    else:
        marker = f'unsigned {"int" if width == 16 else "long"} CRC{width}'
        core = marker + text.split(marker, 1)[1].split('\nstatic unsigned', 1)[0]
    return '/* Kernel from c-bench-64; only benchmark/UI wrappers removed. */\n#define __data\n' + core.strip() + '\n'


def oracle(data, width):
    # Independent byte-table reference, with published check values below.
    poly = {8: 0x1d, 16: 0x1021, 32: 0x04c11db7}[width]
    mask = (1 << width) - 1
    table = []
    for byte in range(256):
        remainder = byte << (width - 8)
        for _ in range(8):
            remainder = ((remainder << 1) ^ (poly if remainder >> (width - 1) else 0)) & mask
        table.append(remainder)
    crc = 0
    for byte in data:
        crc = ((crc << 8) & mask) ^ table[(crc >> (width - 8)) ^ byte]
    return crc ^ (mask if width == 32 else 0)


def vectors():
    state = 0x8162026
    random = []
    for _ in range(8192):
        state ^= (state << 13) & 0xffffffff
        state ^= state >> 17
        state ^= (state << 5) & 0xffffffff
        random.append(state & 255)
    data = [('empty', []), ('zero', [0]), ('high-bit', [0x80]), ('all-bits', [255]),
            ('check', list(b'123456789')), ('ascending-256', list(range(256))),
            ('random-257', random[:257]), ('benchmark-8192', random)]
    for width, expected in [(8, 0x37), (16, 0x31c3), (32, 0x765e7680)]:
        assert oracle(b'123456789', width) == expected
    return [dict(name=name + ('-cross-bank' if cross else ''), address=0x12fff9 if cross else 0x12e000,
                 data=payload, expected={str(w): oracle(payload, w) for w in (8, 16, 32)})
            for name, payload in data for cross in (False, True)]


def calypsi_inventory(directory):
    text = (directory / 'code.map').read_text().replace('\r\n', '\n')
    entries = re.findall(r"^(\S+) in section '(\w+)'\s+placed at address ([0-9a-f]+)-([0-9a-f]+) of size ([0-9a-f]+)\n"
                         r"\([^\n]+ unit 0 section index (\d+)\)", text, re.M)
    sections = {}
    for name, kind, lo, hi, size, index in entries:
        lo, hi, size = (int(x, 16) for x in (lo, hi, size))
        assert hi + 1 - lo == size and kind == 'farcode'
        sections[int(index)] = dict(name=name, address=lo, size=size)
    code = (directory / 'code.raw').read_bytes()
    assert sum(s['size'] for s in sections.values()) == len(code)
    index, covered, lines = 1, set(), []
    for line in (directory / 'code.lst').read_text().splitlines():
        if re.match(r'^\d+\s+\.section\s', line):
            index += 1
            lines.append('\n; ' + sections[index]['name'])
        match = re.match(r'^\d+\s+([0-9a-f]{6}) ([0-9a-f.]+)\s*(.*)$', line)
        if match:
            offset, encoded, source = match.groups()
            address = sections[index]['address'] + int(offset, 16)
            at, size = address - 0x10000, len(encoded) // 2
            assert len(encoded) % 2 == 0 and 0 <= at < at + size <= len(code)
            assert covered.isdisjoint(range(at, at + size))
            covered.update(range(at, at + size))
            lines.append(f'{address:06X}  {code[at:at+size].hex(" ").upper():<11} {source}')
    assert covered == set(range(len(code)))
    (directory / 'code.linked.lst').write_text('\n'.join(lines) + '\n')
    return list(sections.values())


def verify(manifest):
    hashes = dict(manifest['inputs'])
    hashes.update({v['path']: v['sha256'] for v in manifest['tools'].values()})
    for artifact in manifest['artifacts']:
        hashes.update(artifact['hashes'])
    for path, expected in hashes.items():
        assert digest(path) == expected, f'Changed comparison input: {path}'
    return hashes


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT / 'target/crc65816-comparison')
    parser.add_argument('--suite', type=Path, default=DEFAULT_SUITE)
    parser.add_argument('--actionc', type=Path, default=ROOT / 'target/release/actionc-65816')
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    (out / 'manifest.json').unlink(missing_ok=True)
    tools = dict(actionc=args.actionc.resolve(), calypsi=Path('/usr/local/bin/cc65816').resolve(),
                 assembler=Path('/usr/local/bin/as65816').resolve(), linker=Path('/usr/local/bin/ln65816').resolve())
    calypsi = tools['calypsi'].parent.parent
    registers = calypsi / 'src/lib/lowlevel/pseudoRegisters.s'
    inputs = [*FIXTURES.glob('*'), Path(__file__), ROOT / 'tools/compare65816/build.py',
              ROOT / 'tools/compare65816/calypsi.py', ROOT / 'tools/disassemble65816.py', registers,
              calypsi / 'doc/html/_sources/assembly-interface.rst.txt', calypsi / 'doc/html/_sources/data-storage.rst.txt']
    for width in (8, 16, 32):
        path = args.suite / f'benchmarks/src/crc{width}.c'
        original = path.read_text()
        expected = (FIXTURES / f'crc{width}.c').read_text().replace('\r\n', '\n')
        assert extract(original, width) == expected
        assert extract(original.replace('\r\n', '\n').replace('\n', '\r\n'), width) == expected
        inputs.append(path)
    manifest = dict(schema=1, compiler_revision=run(['git', 'rev-parse', 'HEAD']).strip(),
                    compiler_worktree_status=run(['git', 'status', '--porcelain', '--', 'src', 'Cargo.toml', 'Cargo.lock']),
                    suite_revision=run(['git', '-C', args.suite, 'rev-parse', 'HEAD']).strip(),
                    suite_crc_status=run(['git', '-C', args.suite, 'status', '--porcelain', '--', 'benchmarks/src/crc8.c', 'benchmarks/src/crc16.c', 'benchmarks/src/crc32.c']),
                    versions=dict(calypsi=run([tools['calypsi'], '--version']).strip()),
                    tools={k: dict(path=str(v), sha256=digest(v)) for k, v in tools.items()},
                    inputs={str(p.resolve()): digest(p) for p in inputs if p.is_file()},
                    vectors=vectors(), artifacts=[], crlf_checked=True)
    layout = out / 'layout.json'
    layout.write_text(json.dumps(dict(code_origin=0x10000, data_origin=0x180000, stack_overflow=0x48000, nmi_extra_stack=0, imports=[])))
    linker = out / 'calypsi.scm'
    linker.write_text(LINKER_RULES)
    spec = importlib.util.spec_from_file_location('disassembler', ROOT / 'tools/disassemble65816.py')
    disassembler = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(disassembler)
    for width in (8, 16, 32):
        configurations = [('actionc', 'optimized'), ('calypsi', 'O2-speed')]
        if width == 8:
            configurations.append(('calypsi', 'O1-speed'))
        for compiler, mode in configurations:
            directory = out / f'crc{width}' / (compiler + ('-o1' if mode == 'O1-speed' else ''))
            directory.mkdir(parents=True, exist_ok=True)
            source = FIXTURES / f'crc{width}{".act" if compiler == "actionc" else ".c"}'
            def build(where, path):
                if compiler == 'actionc':
                    commands = [[tools['actionc'], '--layout', layout, '-o', where / 'image.json', path]]
                else:
                    commands = [[tools['calypsi'], '-O1' if mode == 'O1-speed' else '-O2', '--speed', '--data-model', 'huge', '--code-model', 'large',
                                 '--assembly-source', where / 'code.s', '-c', path],
                                [tools['assembler'], '--list-file', where / 'code.lst', '-o', where / 'code.o', where / 'code.s'],
                                [tools['assembler'], '-o', where / 'registers.o', registers],
                                [tools['linker'], '--no-auto-libraries', '--no-data-init-table-section',
                                 '--program-root', f'CRC{width}', '--program-start', f'CRC{width}', '--no-tree-shaking',
                                 '--list-file', where / 'code.map', '--output-format', 'raw', '-o', where / 'code.elf',
                                 where / 'code.o', where / 'registers.o', linker]]
                for index, command in enumerate(commands):
                    run(command, where / f'{index}.log')
                return [list(map(str, c)) for c in commands]
            commands = build(directory, source)
            if compiler == 'actionc':
                image = json.loads((directory / 'image.json').read_text())
                routines = [r for r in image['routines'] if r['name'] == f'CRC{width}']
                assert len(routines) == 1 and not routines[0]['calls']
                guards = [g for s in image['segments'] if s['address'] == routines[0]['address'] for g in image_guard_ranges(image, s)]
                (directory / 'code.linked.lst').write_text(disassembler.disassemble(image))
                artifact = dict(image=str(directory / 'image.json'), arguments=routines[0]['arguments'])
            else:
                routines = calypsi_inventory(directory)
                assert len(routines) == 1 and routines[0]['name'] == f'CRC{width}'
                guards = []
                artifact = dict(binary=str(directory / 'code.raw'))
            crlf = directory / 'crlf'
            crlf.mkdir(exist_ok=True)
            copy = crlf / source.name
            copy.write_bytes(source.read_text().replace('\r\n', '\n').replace('\n', '\r\n').encode())
            build(crlf, copy)
            if compiler == 'actionc':
                assert json.loads((crlf / 'image.json').read_text()) == image
            else:
                calypsi_inventory(crlf)
                assert (crlf / 'code.raw').read_bytes() == (directory / 'code.raw').read_bytes()
            artifact.update(compiler=compiler, mode=mode, width=width, entry=routines[0]['address'], routines=routines,
                            guard_ranges=guards, code_bytes=routines[0]['size'], guard_bytes=sum(b-a for a, b in guards),
                            directory=str(directory), commands=commands,
                            hashes={str(p): digest(p) for p in directory.iterdir() if p.is_file()})
            manifest['artifacts'].append(artifact)
            print(f'CRC{width} {compiler}/{mode}: {artifact["code_bytes"]} bytes ({artifact["guard_bytes"]} guard)', flush=True)
    manifest['inputs'].update({str(p): digest(p) for p in (layout, linker)})
    verify(manifest)
    (out / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')


if __name__ == '__main__':
    main()
