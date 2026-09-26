#!/usr/bin/env python3
"""Build the original byte/bit sieves and equivalent Action! ports for speed."""
import argparse
import importlib.util
import json
from pathlib import Path
import re
import sys

sys.dont_write_bytecode = True
from build import ROOT, digest, image_guard_ranges, run
from crc import verify

FIXTURES = ROOT / 'tools/native65816-runtime-tests/tests/fixtures/sieve_bench'


def extract(source):
    text = source.replace('\r\n', '\n')
    core = text[text.index('#define SIZE'):text.index('\nvoid benchmark(void)')]
    core = core.replace('unsigned int prime_count;\n', '')
    return '/* Original c-bench-64 kernel; benchmark/UI wrappers removed. */\n' + core.strip() + '\n'


def vectors(variant):
    # Independent full-integer Eratosthenes reference; the kernels store odds only.
    limit = 8191 if variant == 'sieve' else 16000
    primes = [True] * (2 * limit + 3)
    primes[0:2] = [False, False]
    for p in range(2, len(primes)):
        if primes[p]:
            for multiple in range(p * p, len(primes), p):
                primes[multiple] = False
    result = []
    for n in [0, 1, 2, 7, 8, 9, 255, 256, 257, 8191] + ([16000] if limit == 16000 else []):
        odds = [int(primes[2 * i + 3]) for i in range(n)]
        if variant == 'sieve':
            flags = odds
        else:
            flags = [255] * (n // 8 + 1)
            for i, prime in enumerate(odds):
                if not prime:
                    flags[i // 8] &= ~(1 << (i & 7))
        count = 1 + sum(odds)  # The original counts 2 even for n=0.
        if n in (8191, 16000):
            assert count == {8191: 1900, 16000: 3432}[n]
        result.append(dict(n=n, expected=count, flags=flags))
    return result


def inventory(directory):
    mapping = (directory / 'code.map').read_text().replace('\r\n', '\n')
    entries = re.findall(r"^(\S+) in section '(\w+)'\s+placed at address ([0-9a-f]+)-([0-9a-f]+) of size ([0-9a-f]+)\n"
                         r"\([^\n]+ unit 0 section index (\d+)\)", mapping, re.M)
    sections = {}
    for name, kind, lo, hi, size, index in entries:
        lo, hi, size = (int(x, 16) for x in (lo, hi, size))
        assert hi + 1 - lo == size and kind in ('farcode', 'chuge', 'zhuge')
        sections[int(index)] = dict(name=name, kind=kind, address=lo, size=size)
    code = (directory / 'code.raw').read_bytes()
    assert sum(s['size'] for s in sections.values() if s['kind'] != 'zhuge') == len(code)
    index, covered, lines = 1, set(), []
    for line in (directory / 'code.lst').read_text().splitlines():
        if re.match(r'^\d+\s+\.section\s', line):
            index += 1
            if index in sections:
                lines.append('\n; ' + sections[index]['name'])
        match = re.match(r'^\d+\s+([0-9a-f]{6}) ([0-9a-f.]+)(?:\s+(.*))?$', line)
        if not match or index not in sections:
            continue
        offset, encoded, source = match.groups()
        address = sections[index]['address'] + int(offset, 16)
        at, size = address - 0x10000, len(encoded) // 2
        assert len(encoded) % 2 == 0 and 0 <= at < at + size <= len(code)
        assert covered.isdisjoint(range(at, at + size))
        covered.update(range(at, at + size))
        lines.append(f'{address:06X}  {code[at:at+size].hex(" ").upper():<11} {source or ""}'.rstrip())
    assert covered == set(range(len(code))), 'unclassified linked bytes'
    (directory / 'code.linked.lst').write_text('\n'.join(lines) + '\n')
    return list(sections.values())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT / 'target/sieve65816-comparison')
    parser.add_argument('--suite', type=Path, default=Path.home() / 'atari/c-bench-64')
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
              ROOT / 'tools/compare65816/crc.py', ROOT / 'tools/disassemble65816.py', registers,
              calypsi / 'doc/html/_sources/assembly-interface.rst.txt']
    for variant in ('sieve', 'sieve_bit'):
        path = args.suite / f'benchmarks/src/{variant}.c'
        original = path.read_text()
        expected = (FIXTURES / f'{variant}.c').read_text().replace('\r\n', '\n')
        assert extract(original) == expected
        assert extract(original.replace('\r\n', '\n').replace('\n', '\r\n')) == expected
        inputs.append(path)
    manifest = dict(schema=1, compiler_revision=run(['git', 'rev-parse', 'HEAD']).strip(),
                    compiler_worktree_status=run(['git', 'status', '--porcelain', '--', 'src', 'Cargo.toml', 'Cargo.lock']),
                    suite_revision=run(['git', '-C', args.suite, 'rev-parse', 'HEAD']).strip(),
                    suite_sieve_status=run(['git', '-C', args.suite, 'status', '--porcelain', '--', 'benchmarks/src/sieve.c', 'benchmarks/src/sieve_bit.c']),
                    versions=dict(calypsi=run([tools['calypsi'], '--version']).strip()),
                    tools={k: dict(path=str(v), sha256=digest(v)) for k, v in tools.items()},
                    inputs={str(p.resolve()): digest(p) for p in inputs if p.is_file()},
                    vectors={v: vectors(v) for v in ('sieve', 'sieve_bit')}, artifacts=[], crlf_checked=True)
    spec = importlib.util.spec_from_file_location('disassembler', ROOT / 'tools/disassemble65816.py')
    decoder = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(decoder)
    for placement, base in [('normal', 0x12e000), ('cross-bank', 0x12fff9)]:
        layout = out / f'{placement}.json'
        layout.write_text(json.dumps(dict(code_origin=0x10000, data_origin=base, stack_overflow=0x48000, nmi_extra_stack=0, imports=[])))
        linker = out / f'{placement}.scm'
        linker.write_text(f"""(define memories
 '((memory Code (address (#x10000 . #x1ffff)) (section farcode cfar chuge))
   (memory Flags (address (#x{base:x} . #x{base+0x3fff:x})) (section zhuge))
   (memory DirectPage (address (#x2000 . #x20ff)) (section registers))
   (base-address _DirectPageStart DirectPage 0)))
""")
        manifest['inputs'].update({str(p): digest(p) for p in (layout, linker)})
        for variant in ('sieve', 'sieve_bit'):
            for compiler, mode in [('actionc', 'optimized'), ('calypsi', 'O2-speed'), ('calypsi', 'O2-speed-no-cross-call')]:
                directory = out / placement / variant / (compiler + ('-no-cross-call' if mode.endswith('no-cross-call') else ''))
                directory.mkdir(parents=True, exist_ok=True)
                source = FIXTURES / (variant + ('.act' if compiler == 'actionc' else '.c'))
                def build(where, path):
                    if compiler == 'actionc':
                        commands = [[tools['actionc'], '--layout', layout, '-o', where / 'image.json', path]]
                    else:
                        commands = [[tools['calypsi'], '-O2', '--speed', *(['--no-cross-call'] if mode.endswith('no-cross-call') else []),
                                     '--data-model', 'huge', '--code-model', 'large',
                                     '--assembly-source', where / 'code.s', '-c', path],
                                    [tools['assembler'], '--list-file', where / 'code.lst', '-o', where / 'code.o', where / 'code.s'],
                                    [tools['assembler'], '-o', where / 'registers.o', registers],
                                    [tools['linker'], '--no-auto-libraries', '--no-data-init-table-section',
                                     '--program-root', 'sieve', '--program-start', 'sieve',
                                     '--list-file', where / 'code.map', '--output-format', 'raw', '-o', where / 'code.elf',
                                     where / 'code.o', where / 'registers.o', linker]]
                    for index, command in enumerate(commands):
                        run(command, where / f'{index}.log')
                    return [list(map(str, c)) for c in commands]
                commands = build(directory, source)
                if compiler == 'actionc':
                    image = json.loads((directory / 'image.json').read_text())
                    routines = [r for r in image['routines'] if r['name'] != 'Main']
                    guards = [g for s in image['segments'] if any(s['address'] == r['address'] for r in routines)
                              for g in image_guard_ranges(image, s)]
                    (directory / 'code.linked.lst').write_text(decoder.disassemble(image))
                    entry = next(r for r in routines if r['name'] == 'Sieve')
                    data = image['data']
                    artifact = dict(image=str(directory / 'image.json'), arguments=entry['arguments'])
                else:
                    sections = inventory(directory)
                    routines = [s for s in sections if s['kind'] == 'farcode']
                    data = [s for s in sections if s['kind'] != 'farcode']
                    guards = []
                    entry = next(r for r in routines if r['name'] == 'sieve')
                    artifact = dict(binary=str(directory / 'code.raw'))
                flags = next(d for d in data if d['name'] == 'flags')
                assert flags['address'] == base and flags['size'] == (8191 if variant == 'sieve' else 2001)
                crlf = directory / 'crlf'
                crlf.mkdir(exist_ok=True)
                copy = crlf / source.name
                copy.write_bytes(source.read_text().replace('\r\n', '\n').replace('\n', '\r\n').encode())
                build(crlf, copy)
                if compiler == 'actionc':
                    assert json.loads((crlf / 'image.json').read_text()) == image
                else:
                    assert inventory(crlf) == sections
                    assert (crlf / 'code.raw').read_bytes() == (directory / 'code.raw').read_bytes()
                artifact.update(compiler=compiler, mode=mode,
                                variant=variant, placement=placement, entry=entry['address'], routines=routines, data=data,
                                guard_ranges=guards, code_bytes=sum(r['size'] for r in routines),
                                guard_bytes=sum(b-a for a, b in guards), rodata_bytes=0 if variant == 'sieve' else 8,
                                directory=str(directory), commands=commands,
                                hashes={str(p): digest(p) for p in directory.iterdir() if p.is_file()})
                manifest['artifacts'].append(artifact)
                print(f'{placement} {variant} {compiler}: {artifact["code_bytes"]} code bytes, {artifact["guard_bytes"]} guard', flush=True)
    verify(manifest)
    (out / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')


if __name__ == '__main__':
    main()
