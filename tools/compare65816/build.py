#!/usr/bin/env python3
"""Build matched 65816 kernels; serialize inputs for the independent VM probe."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
from corpus import DEST, ROOT, files


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def run(command, log=None):
    command = list(map(str, command))
    result = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, text=True, timeout=180)
    if log is not None:
        Path(log).write_text(result.stdout)
    if result.returncode:
        raise RuntimeError(' '.join(command)+'\n'+result.stdout)
    return result.stdout


def check_ranges(code, base):
    """Recognize complete current stack-check sequences, including local targets.

    These ranges only classify existing bytes for accounting; no code is changed.
    Unknown entry sequences fail rather than assuming a fixed overhead.
    """
    def long(value):
        return value.to_bytes(3, 'little')
    ranges = []
    for at in range(len(code)-44):
        amount = code[at+22:at+24]
        address = base+at
        pattern = (bytes.fromhex('3b aa c5 46 b0 04 5c')+long(address+20)
                   +bytes.fromhex('d0 04 5c')+long(address+20)
                   +b'\x5c'+long(address+38)+bytes.fromhex('38 e9')+amount
                   +bytes.fromhex('b0 04 5c')+long(address+38)
                   +bytes.fromhex('c5 44 90 04 5c')+long(address+45)
                   +b'\xa9'+amount+bytes.fromhex('5c 00 80 04'))
        if code[at:at+45] == pattern:
            ranges.append([address, address+45])
    return ranges


def native_name(name, wanted):
    return name == wanted or re.search(r'_'+wanted.upper()+r'_', name.upper())


def linked_listing(directory):
    """Combine vasm instruction boundaries with final, relocated vlink bytes."""
    listing = (directory/'code.lst').read_text()
    mapping = (directory/'code.map').read_text()
    linked = (directory/'code.bin').read_bytes()
    bases = {}
    for section, name in re.findall(r'^([0-9A-Fa-f]+): "([^"]+)"', listing, re.M):
        suffix = name.removeprefix('DONTMERGE')
        match = re.search(r'^\s+([0-9A-Fa-f]+) - [0-9A-Fa-f]+ code.o\(\d+'
                          +re.escape(suffix)+r'\)$', mapping, re.M)
        bases[section] = int(match.group(1), 16)
    lines = []
    covered = set()
    for section, offset, encoded, source in re.findall(
            r'^([0-9A-Fa-f]+):([0-9A-Fa-f]+) ([0-9A-Fa-f]+)\s+\d+:\s*(.*)$', listing, re.M):
        address = bases[section]+int(offset, 16)
        at = address-0x10000
        size = len(bytes.fromhex(encoded))
        assert at >= 0 and at+size <= len(linked)
        assert covered.isdisjoint(range(at, at+size))
        covered.update(range(at, at+size))
        lines.append(f'{address:06X}  {linked[at:at+size].hex(" ").upper():<11} {source}')
    assert covered == set(range(len(linked))), 'unclassified linker bytes'
    (directory/'code.linked.lst').write_text('\n'.join(lines)+'\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT/'target/code-quality-65816')
    parser.add_argument('--actionc', type=Path, default=ROOT/'target/release/actionc-65816')
    parser.add_argument('--vbcc-bin', type=Path,
                        default=Path.home()/'atari/vbcc/bin')
    parser.add_argument('--verify-crlf', action='store_true')
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    # A failed rebuild cannot leave an old build manifest looking current.
    (output/'manifest.json').unlink(missing_ok=True)
    for name, expected in files().items():
        assert (DEST/name).read_text().replace('\r\n', '\n') == expected, name
    compilers = dict(actionc=args.actionc.resolve(),
                     vbcc=(args.vbcc_bin/'vbcc65816').resolve(),
                     vasm=(args.vbcc_bin/'vasm6502_oldstyle').resolve(),
                     vlink=(args.vbcc_bin/'vlink').resolve())
    for path in compilers.values():
        if not path.is_file():
            raise SystemExit(f'Missing tool: {path}')
    banners = {}
    with tempfile.TemporaryDirectory(prefix='compare65816-versions-') as temporary:
        for name in ('vbcc', 'vasm', 'vlink'):
            command = [str(compilers[name]), *(['-v'] if name == 'vlink' else [])]
            result = subprocess.run(command, cwd=temporary, capture_output=True, text=True)
            banners[name] = (result.stdout+result.stderr).strip()
    spec = importlib.util.spec_from_file_location('disassemble65816', ROOT/'tools/disassemble65816.py')
    disassembler = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(disassembler)
    layout = output/'layout.json'
    layout.write_text(json.dumps(dict(code_origin=0x10000, data_origin=0x180000,
                                     stack_overflow=0x48000, nmi_extra_stack=0, imports=[])))
    linker = output/'vbcc.ld'
    linker.write_text('\n'.join([f'r{i} = {i*2};' for i in range(32)]
                               +[f'btmp{i} = {64+i*4};' for i in range(4)])
                      +'\nSECTIONS { .text 0x10000 : { *(*text*) } }\n')
    cases = json.loads((DEST/'cases.json').read_text())
    manifest = dict(schema=1, target='wdc-65816-native', cases=cases, artifacts=[],
                    compiler_revision=run(['git', 'rev-parse', 'HEAD']).strip(),
                    compiler_worktree_status=run(['git', 'status', '--porcelain', '--',
                                                  'src', 'Cargo.toml', 'Cargo.lock']),
                    version_banners=banners,
                    tools={k: dict(path=str(v), sha256=digest(v)) for k, v in compilers.items()},
                    inputs={str(p.relative_to(ROOT)): digest(p) for p in
                            [*DEST.glob('*'), *Path(__file__).parent.glob('*.py')] if p.is_file()},
                    crlf_checked=args.verify_crlf)
    for case in cases:
        for mode in ('raw', 'optimized'):
            for compiler in ('actionc', 'vbcc'):
                directory = output/case['id']/mode/compiler
                directory.mkdir(parents=True, exist_ok=True)
                ext = '.act' if compiler == 'actionc' else '.c'
                source = DEST/(case['id']+ext)

                def build(where, source):
                    commands = []
                    if compiler == 'actionc':
                        command = [compilers['actionc'], '--layout', layout,
                                   '-o', where/'image.json', source]
                        if mode == 'raw':
                            command.insert(1, '--no-opt')
                        commands.append(command)
                    else:
                        commands = [
                            [compilers['vbcc'], '-O='+('0' if mode == 'raw' else '1023'),
                             '-mhuge', '-ptr24', '-near-threshold=0', '-no-near-const',
                             f'-o={where/"code.asm"}', source],
                            [compilers['vasm'], '-816', '-opt-branch', '-Fvobj',
                             '-L', where/'code.lst', '-o', where/'code.o', where/'code.asm'],
                            [compilers['vlink'], '-brawbin1', '-T', linker, '-e', '_Work',
                             f'-M{where/"code.map"}', '-o', where/'code.bin', where/'code.o']]
                    for i, command in enumerate(commands):
                        run(command, where/f'{i}.log')
                    return [list(map(str, c)) for c in commands]

                commands = build(directory, source)
                artifact = dict(case=case['id'], mode=mode, compiler=compiler,
                                directory=str(directory), commands=commands,
                                guard_ranges=[])
                if compiler == 'actionc':
                    image = json.loads((directory/'image.json').read_text())
                    routines = [r for r in image['routines'] if not native_name(r['name'], 'Main')]
                    worker = next(r for r in routines if native_name(r['name'], 'Work'))
                    assert [a['size'] for a in worker['arguments']] == case['args']
                    assert worker['result_bytes'] == case['returns']
                    ranges = [[r['address'], r['address']+r['size']] for r in routines]
                    filtered = dict(image)
                    filtered['segments'] = [s for s in image['segments'] if s['executable']
                                            and any(a == s['address'] for a, _ in ranges)]
                    listing = disassembler.disassemble(filtered)
                    (directory/'code.asm').write_text(listing)
                    for segment in filtered['segments']:
                        guards = check_ranges(bytes(segment['bytes']), segment['address'])
                        assert any(lo == segment['address'] for lo, hi in guards)
                        artifact['guard_ranges'].extend(guards)
                    artifact.update(image=str(directory/'image.json'), entry=worker['address'],
                                    code_ranges=ranges, code_bytes=sum(r['size'] for r in routines),
                                    arguments=worker['arguments'], routines=routines,
                                    static_stack_check_bytes=45*len(artifact['guard_ranges']))
                else:
                    symbols = {name: int(value, 16) for value, name in re.findall(
                        r'^\s+0x([0-9a-fA-F]+) (\S+): global (?:reloc|abs),',
                        (directory/'code.map').read_text().replace('\r\n', '\n'), re.M)}
                    size = (directory/'code.bin').stat().st_size
                    # vlink 0.18a omits a section's symbols when there is only
                    # one. Resolve the assembler's exported offset against the
                    # linker's input-section mapping in that case.
                    if '_Work' not in symbols:
                        listing = (directory/'code.lst').read_text()
                        section, offset = re.search(
                            r'^_Work\s+([0-9A-Fa-f]+):([0-9A-Fa-f]+) EXP$', listing, re.M).groups()
                        section_name = re.search(
                            r'^'+section+r': "([^"]+)"', listing, re.M).group(1)
                        suffix = section_name.removeprefix('DONTMERGE')
                        start = re.search(r'^\s+([0-9A-Fa-f]+) - [0-9A-Fa-f]+ code.o\(\d+'
                                          +re.escape(suffix)+r'\)$',
                                          (directory/'code.map').read_text(), re.M).group(1)
                        symbols['_Work'] = int(start, 16)+int(offset, 16)
                    artifact.update(binary=str(directory/'code.bin'), entry=symbols['_Work'],
                                    code_ranges=[[0x10000, 0x10000+size]], code_bytes=size,
                                    symbols=symbols, static_stack_check_bytes=0,
                                    arguments=[dict(size=size, slot_size=(size+1)&~1)
                                               for size in case['args']])
                    linked_listing(directory)
                if args.verify_crlf:
                    crlf = directory/'crlf'
                    crlf.mkdir(exist_ok=True)
                    copy = crlf/source.name
                    copy.write_bytes(source.read_text().replace('\r\n', '\n')
                                     .replace('\n', '\r\n').encode())
                    build(crlf, copy)
                    if compiler == 'actionc':
                        assert json.loads((crlf/'image.json').read_text()) == image
                    else:
                        assert (crlf/'code.bin').read_bytes() == (directory/'code.bin').read_bytes()
                artifact['hashes'] = {p.name: digest(p) for p in directory.iterdir()
                                      if p.is_file() and p.suffix in ('.json', '.asm', '.bin', '.o', '.map', '.lst')}
                manifest['artifacts'].append(artifact)
        print('Built', case['id'], flush=True)
    (output/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
    print('VM input:', output/'manifest.json')


if __name__ == '__main__':
    main()
