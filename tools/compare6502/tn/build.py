#!/usr/bin/env python3
"""Pinned TN/Oscar64 build and artifact collection (no compiler changes)."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'build/tn-c-oscar64'
OSCAR_REV = '4da3147c1f53bfa543382a7e3febd113ceaec48f'
ACTION_REV = 'd11fe9ca20edfc1975489453d39bf0364c3795ff'


def portable(args):
    return [str(a).replace(str(ROOT)+'/', '') for a in args]


def bootstrap():
    repo = OUT/'oscar64'
    if not repo.exists():
        command(['git','init',repo])
        command(['git','-C',repo,'remote','add','origin','https://github.com/drmortalwombat/oscar64.git'])
        command(['git','-C',repo,'fetch','--depth','1','origin',OSCAR_REV])
        command(['git','-C',repo,'checkout','--detach','FETCH_HEAD'])
    actual = subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()
    if actual != OSCAR_REV:
        raise ValueError(f'Oscar64 revision {actual}, expected {OSCAR_REV}')
    command(['make','-C',repo,'-f','make/makefile','compiler','-j8'],OUT/'oscar-build.log')
    reference=OUT/'actionc-reference'
    if not reference.exists():
        reference.mkdir()
        with tempfile.TemporaryFile() as stream:
            subprocess.run(['git','archive',ACTION_REV],cwd=ROOT,stdout=stream,check=True)
            stream.seek(0)
            with tarfile.open(fileobj=stream) as archive:
                archive.extractall(reference,filter='data')
    command(['cargo','build','--release','--locked','--manifest-path',reference/'Cargo.toml',
             '--bin','actionc','--bin','actionc-emit','--target-dir',OUT/'actionc-target'],
            OUT/'actionc-build.log')


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def command(args, log=None):
    args = list(map(str, args))
    if log:
        with Path(log).open('w') as f:
            subprocess.run(args, cwd=ROOT, stdout=f, stderr=subprocess.STDOUT, check=True)
    else:
        subprocess.run(args, cwd=ROOT, check=True)
    return args


def xex_segments(path):
    data = Path(path).read_bytes()
    pos, result = 0, []
    while pos < len(data):
        if data[pos:pos+2] == b'\xff\xff':
            pos += 2
            continue
        if pos+4 > len(data):
            raise ValueError(f'{path}: truncated segment header')
        start = int.from_bytes(data[pos:pos+2], 'little')
        end = int.from_bytes(data[pos+2:pos+4], 'little') + 1
        pos += 4
        if end <= start or pos+end-start > len(data):
            raise ValueError(f'{path}: invalid segment ${start:04X}..${end:04X}')
        result.append((start, data[pos:pos+end-start]))
        pos += end-start
    return result


def collect(stem):
    image = stem.with_suffix('.xex')
    # Audit the compiler-generated listing, without disassembling the image.
    instructions={int(m[1],16):m[2].split() for m in re.finditer(
        r'^([0-9a-f]{4}) : (.*)',stem.with_suffix('.asm').read_text(),re.M)
        if re.match(r'[0-9a-f]{2} ',m[2])}
    for pc,tokens in instructions.items():
        if tokens[0] in ['10','30','50','70','90','b0','d0','f0']:
            displacement=int(tokens[1],16)
            target=(pc+2+(displacement if displacement<128 else displacement-256))&65535
            assert target in instructions, f'{stem}: branch at ${pc:04X} enters non-instruction ${target:04X}'

    debug = json.loads(stem.with_suffix('.dbj').read_text())
    symbols = {r['name']: r for r in debug['memory']}
    stem.with_suffix('.symbols.tsv').write_text(''.join(
        f"{name}\t{r['start']}\t{r['end']}\t{r['type']}\n"
        for name, r in sorted(symbols.items())))
    # IR headers carry allocated temporary byte offsets. Keep all temporary
    # lanes below the fixed address/stack bank at $D5; assembly's 32-byte
    # saved-register restore uses $B5..$D4 independently.
    maximum=0
    for offset,kind in re.findall(r'\$([0-9a-fA-F]+) R\d+\(([A-Z])\)',stem.with_suffix('.int').read_text()):
        width={'C':1,'B':1,'I':2,'P':2,'L':4,'F':4}.get(kind)
        if width is None:raise ValueError(f'unknown IR temporary width {kind}')
        maximum=max(maximum,int(offset,16)+width)
    assert 0xA5+maximum<=0xD5, f'compiler temporary bank overlaps fixed registers: {maximum}'
    segments = xex_segments(image)
    assert any(a == 0x2E0 and len(b) == 2 for a, b in segments), 'XEX needs RUNAD'
    occupied = set()
    for start, data in segments:
        assert start == 0x2E0 or start >= 0x2C00
        for a in range(start, start+len(data)):
            assert a not in occupied, f'overlapping XEX segment at ${a:04X}'
            occupied.add(a)
    return dict(image=str(image.relative_to(ROOT)), sha256=digest(image),
                load_file_bytes=image.stat().st_size,
                segments=[dict(start=a, bytes=len(b)) for a, b in segments],
                code_bytes=sum(r['end']-r['start'] for r in debug['memory']
                               if r['type'] == 'NATIVE_CODE'),
                bss_end=symbols['BSSEnd']['start'], stack_end=symbols['StackEnd']['start'],temporary_bytes=maximum,compiler_sp=0xD9)


def compile_c(compiler, source, stem, profile, defines=()):
    stem.parent.mkdir(parents=True, exist_ok=True)
    args = [compiler, '-n', '-tm=atari', '-'+profile, '-g',
            '-i='+str(ROOT/'samples/tn/c'), *defines,
            '-o='+str(stem.with_suffix('.xex')), ROOT/source]
    command(args, stem.with_suffix('.log'))
    row = collect(stem)
    row['command'] = portable(args)
    with tempfile.TemporaryDirectory(dir=OUT,prefix='c-crlf-') as temp:
        temp=Path(temp)
        for file in (ROOT/'samples/tn/c').iterdir():
            if file.suffix in ['.c','.h']:
                text=file.read_text().replace('\r\n','\n')
                (temp/file.name).write_bytes(text.replace('\n','\r\n').encode())
        text=(ROOT/source).read_text().replace('\r\n','\n')
        (temp/Path(source).name).write_bytes(text.replace('\n','\r\n').encode())
        crlf=temp/'check.xex'
        command([compiler,'-n','-tm=atari','-'+profile,'-g','-i='+str(temp),*defines,
                 '-o='+str(crlf),temp/Path(source).name],temp/'check.log')
        assert digest(crlf)==row['sha256'], f'actual CRLF image differs for {source}/{profile}'
    row['actual_crlf_equal']=True
    return row


def inventory():
    rows = []
    for name in ['LIB.ACT', 'TN.ACT', 'TNDBG.ACT']:
        p = ROOT/'samples/tn/modern'/name
        for line, text in enumerate(p.read_text().splitlines(), 1):
            m = re.match(r'\s*(?:(BYTE|CHAR|CARD|INT)\s+)?(PROC|FUNC)\s+(\w+)', text)
            if m:
                rows.append(dict(source=str(p.relative_to(ROOT)), line=line, routine=m[3],
                                 kind='machine/data' if '=*' in text else 'high-level',
                                 status='pending', implementation=None, qualification=[]))
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('task', choices=['bootstrap', 'probe', 'platform', 'kernels', 'panels', 'ui', 'dispatch', 'freeze'])
    parser.add_argument('--oscar', type=Path, default=Path(os.environ.get(
        'OSCAR64', OUT/'oscar64/bin/oscar64')))
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    compiler = args.oscar.resolve()
    if args.task == 'bootstrap':
        bootstrap()
        return
    if args.task == 'dispatch':
        rows=[]
        for profile in ['Os','O3']:
            for debug in [False,True]:
                defines=['-dTN_DIAGNOSTIC=1','-dTN_WITH_PANELS=1','-dTN_WITH_UI=1','-dTN_WITH_DISPATCH=1']
                if debug:defines+=['-dTN_DEBUG=1']
                rows.append(compile_c(compiler,'tools/compare6502/tn/dispatch_probe.c',
                                     OUT/(f'dispatch-{profile}'+('-debug' if debug else '')),profile,defines))
        (OUT/'dispatch.json').write_text(json.dumps(rows,indent=2)+'\n')
        return
    if args.task == 'ui':
        rows=[compile_c(compiler,'tools/compare6502/tn/ui_probe.c',
                        OUT/f'ui-{profile}',profile,['-dTN_DIAGNOSTIC=1','-dTN_WITH_PANELS=1','-dTN_WITH_UI=1']) for profile in ['Os','O3']]
        (OUT/'ui.json').write_text(json.dumps(rows,indent=2)+'\n')
        return
    if args.task == 'panels':
        rows=[compile_c(compiler,'tools/compare6502/tn/panel_probe.c',
                        OUT/f'panels-{profile}',profile,['-dTN_DIAGNOSTIC=1','-dTN_WITH_PANELS=1']) for profile in ['Os','O3']]
        (OUT/'panels.json').write_text(json.dumps(rows,indent=2)+'\n')
        return
    if args.task == 'kernels':
        rows=[compile_c(compiler,'tools/compare6502/tn/kernel_probe.c',
                        OUT/f'kernels-{profile}',profile,['-dTN_DIAGNOSTIC=1']) for profile in ['Os','O3']]
        (OUT/'kernels.json').write_text(json.dumps(rows,indent=2)+'\n')
        return
    if args.task == 'platform':
        rows=[compile_c(compiler,'tools/compare6502/tn/platform_probe.c',
                        OUT/f'platform-{profile}',profile) for profile in ['Os','O3']]
        (OUT/'platform.json').write_text(json.dumps(rows,indent=2)+'\n')
        return
    if args.task == 'probe':
        rows = [compile_c(compiler, 'tools/compare6502/tn/probe.c',
                          OUT/f'probe-{profile}', profile) for profile in ['Os','O3']]
        with tempfile.TemporaryDirectory(dir=OUT, prefix='probe-crlf-') as temp:
            temp=Path(temp)
            for name in ['tn_target.h','tn_types.h']:
                text=(ROOT/'samples/tn/c'/name).read_text().replace('\r\n','\n')
                (temp/name).write_bytes(text.replace('\n','\r\n').encode())
            source=(ROOT/'tools/compare6502/tn/probe.c').read_text().replace('\r\n','\n')
            (temp/'probe.c').write_bytes(source.replace('\n','\r\n').encode())
            for profile in ['Os','O3']:
                stem=temp/f'probe-{profile}'
                command([compiler,'-n','-tm=atari','-'+profile,'-g','-i='+str(temp),
                         '-o='+str(stem.with_suffix('.xex')),temp/'probe.c'],stem.with_suffix('.log'))
                assert digest(stem.with_suffix('.xex')) == digest(OUT/f'probe-{profile}.xex')
        manifest = dict(oscar_revision=OSCAR_REV, compiler_sha256=digest(compiler),
                        actual_crlf_equal=True, artifacts=rows)
        (OUT/'probe.json').write_text(json.dumps(manifest, indent=2)+'\n')
    else:
        binary = OUT/'actionc-target/release/actionc'
        rows = []
        for runtime in ['cart','standalone']:
            for name in ['TN','TNDBG']:
                stem = OUT/'reference'/f'{name}-{runtime}'
                stem.parent.mkdir(exist_ok=True)
                source = OUT/'actionc-reference/samples/tn/modern'/f'{name}.ACT'
                cmd = command([binary, '--mode','mir6502','--runtime',runtime,
                               '--listing',stem.with_suffix('.asm'),'-o',stem.with_suffix('.xex'),source],
                              stem.with_suffix('.log'))
                rows.append(dict(name=name,runtime=runtime,command=portable(cmd),
                                 bytes=stem.with_suffix('.xex').stat().st_size,
                                 sha256=digest(stem.with_suffix('.xex'))))
        paths = ['samples/tn/modern/'+n for n in ['TN.ACT','TNDBG.ACT','LIB.ACT']]
        paths += ['roms/altirraos-xl.rom','roms/action.rom','atr/mydos.atr',
                  'tools/vm-runtime-tests/Cargo.lock']
        for p in paths[:3]:
            assert digest(ROOT/p)==digest(OUT/'actionc-reference'/p), 'reference source drift'
        manifest = dict(actionc_revision=ACTION_REV, actionc_binary_sha256=digest(binary),
                        oscar_revision=OSCAR_REV, oscar_binary_sha256=digest(compiler),
                        inputs={p:digest(ROOT/p) for p in paths}, artifacts=rows)
        (ROOT/'surveys/tn/c-oscar64/baseline.json').write_text(json.dumps(manifest,indent=2)+'\n')
        inventory_path=ROOT/'surveys/tn/c-oscar64/inventory.json'
        if not inventory_path.exists():
            inventory_path.write_text(json.dumps(inventory(),indent=2)+'\n')
        print(json.dumps(rows,indent=2))


if __name__ == '__main__':
    main()
