#!/usr/bin/env python3
"""Build the larger Dijkstra comparison, without changing the small kernel corpus."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys
import time
import urllib.request
import zipfile

sys.dont_write_bytecode = True
from build import ROOT, image_guard_ranges, digest, run

HERE = Path(__file__).resolve().parent
FIXTURE = ROOT / 'fixtures/runtime/tacle/dijkstra'
URL = 'http://www.ibaug.de/vbcc/vbcc65816_r2.zip'
ARCHIVE_SHA = 'e8950454d55327e50f6a5f98dc22ac020ee3d6084a17c92d37e088d3c63a922c'
LIBRARY = 'vbcc65816/vbcc65816_linux/vbcc/targets/65816-sim/lib/libvch.a'


def replace_once(text, before, after):
    assert text.count(before) == 1, repr(before)
    return text.replace(before, after)


def sources(read=lambda name: (FIXTURE / name).read_text()):
    text = lambda name: read(name).replace('\r\n', '\n')
    core = replace_once(text('kernel.inc'), 'j=NumNodes/2', 'j=50')
    core = replace_once(core, '    j=j MOD NumNodes\n', '')
    source = text('dijkstra.c')
    assert hashlib.sha256(source.encode()).hexdigest() == 'ffa37cd0725262ef946bc1946ca39f168d19d95b816b6483939b322bd44df43c'
    source = source[:source.index('int main( void )\n{')]
    source = replace_once(source, 'int main( void );', '')
    source = replace_once(source, '#include "input.h"',
                          '#define NUM_NODES 100\nunsigned char matrix[NUM_NODES][NUM_NODES];')
    source = re.sub(r'_Pragma\(\s*"[^"]*"\s*\)', '', source)
    names = dict(dijkstra_rgnNodes='nodes', dijkstra_queueCount='queueCount',
                 dijkstra_queueNext='queueNext', dijkstra_queueHead='head',
                 dijkstra_queueItems='items', dijkstra_checksum='checksum',
                 dijkstra_AdjMatrix='matrix', dijkstra_init='Init',
                 dijkstra_return='ChecksumResult', dijkstra_enqueue='Enqueue',
                 dijkstra_dequeue='Dequeue', dijkstra_qcount='QueueLength',
                 dijkstra_find='Find', dijkstra_main='Benchmark')
    for old, new in names.items():
        source = re.sub(r'\b'+old+r'\b', new, source)
    source = replace_once(source, 'int checksum = 0;', 'int checksum;')
    source = replace_once(source, 'j = NUM_NODES / 2', 'j = 50')
    source = replace_once(source, '    j = j % NUM_NODES;', '')
    source += '\n'+(HERE / 'dijkstra_wrapper.c').read_text().replace('\r\n', '\n')
    return {'dijkstra.act': text('dijkstra.act'), 'types.inc': text('types.inc'),
            'kernel.inc': core, 'dijkstra.c': source}


def linked_listing(directory, routines):
    """Use assembler boundaries with relocated bytes; mark library bytes explicitly."""
    listing = (directory/'code.lst').read_text()
    mapping = (directory/'code.map').read_text()
    code = (directory/'code.bin').read_bytes()
    bases = {}
    for section, name in re.findall(r'^([0-9A-Fa-f]+): "([^"]+)"', listing, re.M):
        match = re.search(r'^\s+([0-9A-Fa-f]+) - [0-9A-Fa-f]+ code.o\(\d+'
                          + re.escape(name.removeprefix('DONTMERGE'))+r'\)$', mapping, re.M)
        assert match, name
        bases[section] = int(match[1], 16)
    lines, covered = [], set()
    for section, offset, encoded, source in re.findall(
            r'^([0-9A-Fa-f]+):([0-9A-Fa-f]+) ([0-9A-Fa-f]+)\s+\d+:\s*(.*)$', listing, re.M):
        address = bases[section]+int(offset, 16)
        if address >= 0x180000:  # BSS reservation lines are not file bytes.
            continue
        at, size = address-0x10000, len(bytes.fromhex(encoded))
        assert 0 <= at < at+size <= len(code)
        assert covered.isdisjoint(range(at, at+size))
        covered.update(range(at, at+size))
        lines.append((address, f'{address:06X}  {code[at:at+size].hex(" ").upper():<11} {source}'))
    for routine in routines:
        if not routine['library']:
            continue
        lo, hi = routine['address']-0x10000, routine['address']-0x10000+routine['size']
        assert covered.isdisjoint(range(lo, hi))
        covered.update(range(lo, hi))
        for at in range(lo, hi, 8):
            address = at+0x10000
            lines.append((address, f'{address:06X}  {code[at:min(at+8,hi)].hex(" ").upper()} ; {routine["name"]} library bytes (not disassembled)'))
    assert covered == set(range(len(code))), 'unclassified linked bytes'
    (directory/'code.linked.lst').write_text('\n'.join(line for _, line in sorted(lines))+'\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT/'target/dijkstra-65816')
    parser.add_argument('--actionc', type=Path, default=ROOT/'target/release/actionc-65816')
    parser.add_argument('--vbcc-bin', type=Path, default=Path.home()/'atari/vbcc/bin')
    parser.add_argument('--runtime-archive', type=Path)
    parser.add_argument('--verify-crlf', action='store_true')
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    (output/'manifest.json').unlink(missing_ok=True)
    tools = dict(actionc=args.actionc.resolve(), vbcc=(args.vbcc_bin/'vbcc65816').resolve(),
                 vasm=(args.vbcc_bin/'vasm6502_oldstyle').resolve(), vlink=(args.vbcc_bin/'vlink').resolve())
    inputs = [p for p in FIXTURE.iterdir() if p.is_file()] + [Path(__file__), HERE/'dijkstra_wrapper.c', HERE/'build.py']
    hashes = {str(p): digest(p) for p in inputs}
    tool_hashes = {str(p): digest(p) for p in tools.values()}
    archive = args.runtime_archive or output/'vbcc65816_r2.zip'
    if not archive.exists():
        with urllib.request.urlopen(URL, timeout=60) as response:
            archive.write_bytes(response.read())
    assert digest(archive) == ARCHIVE_SHA, 'runtime archive does not match pinned release'
    library = output/'libvch.a'
    with zipfile.ZipFile(archive) as files:
        library.write_bytes(files.read(LIBRARY))
    generated = sources()
    source_dir = output/'source'
    source_dir.mkdir(exist_ok=True)
    for name, text in generated.items():
        (source_dir/name).write_text(text)
    layout = output/'layout.json'
    layout.write_text(json.dumps(dict(code_origin=0x10000, data_origin=0x180000,
                                     stack_overflow=0x48000, nmi_extra_stack=0, imports=[])))
    linker = output/'vbcc.ld'
    linker.write_text('\n'.join([f'r{i} = {2*i};' for i in range(32)]
                               + [f'btmp{i} = {64+4*i};' for i in range(4)])
                      +'\nSECTIONS { .text 0x10000 : { *(*text*) } .data 0x180000 : { *(*data*) } .bss (NOLOAD) : { *(*bss*) *(COMMON) } }\n')
    spec = importlib.util.spec_from_file_location('disassembler', ROOT/'tools/disassemble65816.py')
    disassembler = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(disassembler)
    manifest = dict(schema=1, compiler_revision=run(['git', 'rev-parse', 'HEAD']).strip(),
                    compiler_worktree_status=run(['git', 'status', '--porcelain', '--', 'src', 'Cargo.toml', 'Cargo.lock']),
                    tools={k: dict(path=str(p), sha256=digest(p)) for k, p in tools.items()},
                    inputs=hashes, crlf_checked=args.verify_crlf,
                    runtime=dict(url=URL, archive_sha256=ARCHIVE_SHA, member=LIBRARY, sha256=digest(library)),
                    vectors=str(FIXTURE/'vectors.txt'), graphs=str(FIXTURE/'graphs.txt'), artifacts=[])
    for compiler in ('actionc', 'vbcc'):
        for mode in ('raw', 'optimized'):
            directory = output/compiler/mode
            directory.mkdir(parents=True, exist_ok=True)

            def build(where, src):
                if compiler == 'actionc':
                    commands = [[tools['actionc'], *(['--no-opt'] if mode == 'raw' else []),
                                 '--layout', layout, '-o', where/'image.json', src/'dijkstra.act']]
                else:
                    commands = [[tools['vbcc'], '-O='+('0' if mode == 'raw' else '1023'),
                                 '-mhuge', '-ptr24', '-near-threshold=0', '-no-near-const',
                                 f'-o={where/"code.asm"}', src/'dijkstra.c'],
                                [tools['vasm'], '-816', '-opt-branch', '-Fvobj', '-L', where/'code.lst', '-o', where/'code.o', where/'code.asm'],
                                [tools['vlink'], '-brawbin1', '-T', linker, '-e', '_Main', f'-M{where/"code.map"}', '-o', where/'code.bin', where/'code.o', library]]
                durations = []
                for i, command in enumerate(commands):
                    start = time.perf_counter()
                    run(command, where/f'{i}.log')
                    durations.append(time.perf_counter()-start)
                return [list(map(str, c)) for c in commands], durations

            commands, durations = build(directory, source_dir)
            artifact = dict(compiler=compiler, mode=mode, directory=str(directory), commands=commands,
                            build_seconds=durations, guard_ranges=[])
            if compiler == 'actionc':
                path = directory/'image.json'
                image = json.loads(path.read_text())
                routines = image['routines']
                for s in image['segments']:
                    if s['executable']:
                        guards = image_guard_ranges(image, s)
                        assert any(lo == s['address'] for lo, _ in guards)
                        artifact['guard_ranges'].extend(guards)
                artifact.update(image=str(path), entry=image['entry'], routines=routines,
                                code_bytes=sum(r['size'] for r in routines),
                                globals={d['name']: dict(address=d['address'], size=d['size']) for d in image['data'] if d['kind']=='global'},
                                data_ranges=[[z['address'], z['address']+z['size']] for z in image['zero_fill']])
                assert not any(not s['executable'] for s in image['segments'])
                (directory/'code.linked.lst').write_text(disassembler.disassemble(image))
            else:
                mapping = (directory/'code.map').read_text()
                symbols = {name: int(value,16) for value,name in re.findall(r'^\s+0x([0-9a-fA-F]+) (\S+): global (?:reloc|abs),', mapping,re.M)}
                entries = re.findall(r'^\s+([0-9a-fA-F]+) - ([0-9a-fA-F]+) (\w+\.o)\(\d+_(text|bss)\.(?:far|huge)\.(\w+)(?:\.0)?\)$', mapping,re.M)
                routines = [dict(name=name, address=int(lo,16), size=int(hi,16)-int(lo,16), library=obj!='code.o') for lo,hi,obj,kind,name in entries if kind=='text']
                globals_ = {name: dict(address=int(lo,16),size=int(hi,16)-int(lo,16)) for lo,hi,_,kind,name in entries if kind=='bss'}
                size = (directory/'code.bin').stat().st_size
                assert sum(r['size'] for r in routines)==size
                assert all(symbols['_'+k] == v['address'] for k,v in globals_.items())
                artifact.update(binary=str(directory/'code.bin'), entry=symbols['_Main'], routines=routines,
                                code_bytes=size, globals=globals_, data_ranges=[[v['address'],v['address']+v['size']] for v in globals_.values()])
                linked_listing(directory,routines)
            artifact['code_ranges'] = [[r['address'],r['address']+r['size']] for r in artifact['routines']]
            artifact['data_bytes'] = sum(b-a for a,b in artifact['data_ranges'])
            if args.verify_crlf:
                crlf = directory/'crlf'
                crlf.mkdir(exist_ok=True)
                for name,text in generated.items():
                    (crlf/name).write_bytes(text.replace('\n','\r\n').encode())
                assert sources(lambda name: (FIXTURE/name).read_text().replace('\n','\r\n')) == generated
                build(crlf, crlf)
                if compiler == 'actionc':
                    assert json.loads((crlf/'image.json').read_text()) == image
                else:
                    assert (crlf/'code.bin').read_bytes() == (directory/'code.bin').read_bytes()
            artifact['hashes'] = {str(p):digest(p) for p in directory.iterdir() if p.is_file()}
            manifest['artifacts'].append(artifact)
            print(compiler,mode,artifact['code_bytes'],'code bytes',flush=True)
    assert all(digest(Path(p))==h for p,h in (hashes|tool_hashes).items()), 'inputs changed while building'
    manifest['generated'] = {str(p):digest(p) for p in [*source_dir.iterdir(),layout,linker,library]}
    (output/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    print(output/'manifest.json')


if __name__ == '__main__':
    main()
