#!/usr/bin/env python3
"""Build and inventory the frozen Exec816 list primitives against equivalent C."""
import argparse
import importlib.util
import json
from pathlib import Path
import re
import sys

sys.dont_write_bytecode = True
from build import ROOT, digest, image_guard_ranges, linked_listing, native_name, run
from execlists_vectors import cases
from calypsi import LINKER_RULES, inventory as calypsi_inventory

FIXTURE = ROOT/'tools/native65816-runtime-tests/tests/fixtures/execlists'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT/'target/execlists-comparison')
    parser.add_argument('--actionc', type=Path, default=ROOT/'target/release/actionc-65816')
    parser.add_argument('--vbcc-bin', type=Path, default=Path.home()/'atari/vbcc/bin')
    parser.add_argument('--calypsi-bin', type=Path, default=Path('/usr/local/bin'))
    args = parser.parse_args()
    out = args.output.resolve(); out.mkdir(parents=True, exist_ok=True)
    (out/'manifest.json').unlink(missing_ok=True)
    tools = dict(actionc=args.actionc.resolve(), vbcc=(args.vbcc_bin/'vbcc65816').resolve(),
                 vasm=(args.vbcc_bin/'vasm6502_oldstyle').resolve(), vlink=(args.vbcc_bin/'vlink').resolve(),
                 calypsi=(args.calypsi_bin/'cc65816').resolve(), calypsi_as=(args.calypsi_bin/'as65816').resolve(),
                 calypsi_link=(args.calypsi_bin/'ln65816').resolve())
    calypsi_root=tools['calypsi'].parent.parent
    registers=calypsi_root/'src/lib/lowlevel/pseudoRegisters.s'
    inputs = {str(p.resolve()):digest(p) for p in [*FIXTURE.iterdir(), Path(__file__),
        Path(__file__).with_name('execlists_vectors.py'), Path(__file__).with_name('calypsi.py'),
        registers, calypsi_root/'include/stddef.h', calypsi_root/'doc/pdf/Calypsi65816Guide.pdf'] if p.is_file()}
    manifest = dict(schema=1, target='wdc-65816-native', cases=cases(), artifacts=[],
                    observe_control_flow=False, compiler_revision=run(['git','rev-parse','HEAD']).strip(),
                    compiler_worktree_status=run(['git','status','--porcelain','--','src','Cargo.toml','Cargo.lock']),
                    tools={k:dict(path=str(p),sha256=digest(p)) for k,p in tools.items()}, inputs=inputs, crlf_checked=True)
    layout = out/'layout.json'
    layout.write_text(json.dumps(dict(code_origin=0x10000, data_origin=0x180000, stack_overflow=0x48000, nmi_extra_stack=0, imports=[])))
    linker = out/'vbcc.ld'
    linker.write_text('\n'.join([f'r{i} = {2*i};' for i in range(32)]+[f'btmp{i} = {64+4*i};' for i in range(4)])+'\nSECTIONS { .text 0x10000 : { *(*text*) } }\n')
    calypsi_linker=out/'calypsi.scm';calypsi_linker.write_text(LINKER_RULES)
    spec = importlib.util.spec_from_file_location('disassembler',ROOT/'tools/disassemble65816.py')
    decoder = importlib.util.module_from_spec(spec);spec.loader.exec_module(decoder)
    sizes = []
    for mode in ['raw','optimized']:
        compiled = {}
        for compiler in ['actionc','vbcc','calypsi']:
            directory = out/compiler/mode;directory.mkdir(parents=True,exist_ok=True)
            def build(where, source):
                if compiler=='actionc':
                    commands=[[tools[compiler],*(['--no-opt'] if mode=='raw' else []),'--layout',layout,'-o',where/'image.json',source/'main.act']]
                elif compiler=='vbcc':
                    commands=[[tools['vbcc'],'-O='+('0' if mode=='raw' else '1023'),'-mhuge','-ptr24','-near-threshold=0','-no-near-const',f'-o={where/"code.asm"}',source/'execlists.c'],
                              [tools['vasm'],'-816','-opt-branch','-Fvobj','-L',where/'code.lst','-o',where/'code.o',where/'code.asm'],
                              [tools['vlink'],'-brawbin1','-T',linker,'-e','_NewList',f'-M{where/"code.map"}','-o',where/'code.bin',where/'code.o']]
                else:
                    commands=[[tools['calypsi'],'-O0' if mode=='raw' else '-O2','--space','--data-model','huge','--code-model','large',
                               '--assembly-source',where/'code.s','-c',source/'execlists.c'],
                              [tools['calypsi_as'],'--list-file',where/'code.lst','-o',where/'code.o',where/'code.s'],
                              [tools['calypsi_as'],'-o',where/'registers.o',registers],
                              [tools['calypsi_link'],'--no-auto-libraries','--no-data-init-table-section',
                               '--program-root','NewList','--program-start','NewList','--no-tree-shaking',
                               '--list-file',where/'code.map','--output-format','raw','-o',where/'code.elf',
                               where/'code.o',where/'registers.o',calypsi_linker]]
                for i,command in enumerate(commands):run(command,where/f'{i}.log')
                return [list(map(str,c)) for c in commands]
            commands=build(directory,FIXTURE)
            if compiler=='actionc':
                image=json.loads((directory/'image.json').read_text());routines=[r for r in image['routines'] if 'EXECLISTS' in r['name']]
                guard_ranges=[g for s in image['segments'] if any(r['address']==s['address'] for r in routines) for g in image_guard_ranges(image,s)]
                (directory/'code.linked.lst').write_text(decoder.disassemble(image))
                base=dict(image=str(directory/'image.json'))
            elif compiler=='vbcc':
                mapping=(directory/'code.map').read_text().replace('\r\n','\n')
                entries=re.findall(r'^\s+([0-9a-fA-F]+) - ([0-9a-fA-F]+) code.o\(\d+_text.far.(\w+).0\)$',mapping,re.M)
                routines=[dict(name=name,address=int(lo,16),size=int(hi,16)-int(lo,16)) for lo,hi,name in entries]
                assert sum(r['size'] for r in routines)==(directory/'code.bin').stat().st_size
                guard_ranges=[];base=dict(binary=str(directory/'code.bin'));linked_listing(directory)
            else:
                routines=calypsi_inventory(directory)
                guard_ranges=[];base=dict(binary=str(directory/'code.bin'))
            crlf=directory/'crlf';crlf.mkdir(exist_ok=True)
            for p in FIXTURE.iterdir():
                if p.is_file(): (crlf/p.name).write_bytes(p.read_text().replace('\r\n','\n').replace('\n','\r\n').encode())
            build(crlf,crlf)
            if compiler=='calypsi': calypsi_inventory(crlf)
            if compiler=='actionc': assert json.loads((crlf/'image.json').read_text())==image
            else: assert (crlf/'code.bin').read_bytes()==(directory/'code.bin').read_bytes()
            hashes={p.name:digest(p) for p in directory.iterdir() if p.is_file() and p.suffix in ['.json','.asm','.s','.bin','.raw','.elf','.o','.map','.lst']}
            compiled[compiler]=(routines,guard_ranges)
            for case in manifest['cases']:
                routine=next(r for r in routines if native_name(r['name'],case['id']))
                guards=[g for g in guard_ranges if routine['address']<=g[0]<routine['address']+routine['size']]
                if compiler=='actionc':
                    assert [a['size'] for a in routine['arguments']]==case['args'];assert routine['result_bytes']==case['returns']
                artifact=base|dict(case=case['id'],mode=mode,compiler=compiler,directory=str(directory),commands=commands,hashes=hashes,
                    entry=routine['address'],code_bytes=routine['size'],static_stack_check_bytes=sum(b-a for a,b in guards),
                    # Dynamic metrics include callees; static code size is the entry routine only.
                    code_ranges=[[r['address'],r['address']+r['size']] for r in routines],guard_ranges=guard_ranges,
                    arguments=routine['arguments'] if compiler=='actionc' else [dict(size=4 if compiler=='calypsi' else w,slot_size=(w+1)&~1) for w in case['args']],routines=routines)
                manifest['artifacts'].append(artifact)
        for name in [c['id'] for c in manifest['cases']]:
            a=next(r for r in compiled['actionc'][0] if native_name(r['name'],name));b=next(r for r in compiled['vbcc'][0] if native_name(r['name'],name))
            guards=sum(hi-lo for lo,hi in compiled['actionc'][1] if a['address']<=lo<a['address']+a['size'])
            c=next(r for r in compiled['calypsi'][0] if native_name(r['name'],name))
            sizes.append(dict(mode=mode,routine=name,action_bytes=a['size'],action_guard_bytes=guards,action_body_bytes=a['size']-guards,vbcc_bytes=b['size'],calypsi_bytes=c['size'],action_frame=a['fixed_frame']))
        shared=sum(r['size'] for r in compiled['calypsi'][0] if r['name'] not in [c['id'] for c in manifest['cases']])
        sizes.append(dict(mode=mode,routine='Shared helpers',action_bytes=0,action_guard_bytes=0,action_body_bytes=0,vbcc_bytes=0,calypsi_bytes=shared,action_frame=0))
    assert all(digest(p)==h for p,h in inputs.items())
    (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    (out/'sizes.json').write_text(json.dumps(sizes,indent=2)+'\n')
    print(f'{len(manifest["artifacts"])} entries; {sum(len(c["vectors"]) for c in manifest["cases"])} vectors; LF/CRLF identical')


if __name__=='__main__':main()
