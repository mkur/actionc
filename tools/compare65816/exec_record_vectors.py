#!/usr/bin/env python3
"""Build actual frozen ExecLists and an independent mixed record-flow workload."""
import argparse
from copy import deepcopy
import json
from pathlib import Path
import shutil
import subprocess
import sys

sys.dont_write_bytecode = True
from exec_record_baseline import PROFILES, digest, read, save, verify_inputs, guard_ranges
from execlists_vectors import cases


def flow_cases():
    vectors = []
    for placement, addresses in [('far', [0x21fffe, 0x320201, 0x430301]),
                                 ('bank_zero', [0x8100, 0x8301, 0x8502])]:
        for count in (1, 2, 3):
            for choose in (0, 1):
                before = {at: bytearray([0xa5] * 14) for at in addresses[:count]}
                for index, at in enumerate(before):
                    row = before[at]
                    row[0] = (1, 254, 3)[index]
                    row[2:4] = (65532 if index == 0 else index * 101).to_bytes(2, 'little')
                    next_at = addresses[index + 1] if index + 1 < count else 0
                    row[4:7] = next_at.to_bytes(3, 'little')
                after = deepcopy(before)
                total = (int.from_bytes(before[addresses[0]][2:4], 'little') + (0 if choose else 7)) & 65535
                for row in after.values():
                    value = (int.from_bytes(row[2:4], 'little') + (3 if row[0] & 1 else 2)) & 65535
                    row[2:4] = value.to_bytes(2, 'little')
                    total = (total + row[0] + value) & 65535
                    row[9] = total & 255
                encode = lambda memory: [dict(address=at, bytes=list(row)) for at, row in sorted(memory.items())]
                vectors.append(dict(args=[addresses[0], choose], result=total,
                    memory=encode(before), after=encode(after), placement=placement, count=count, choose=choose))
    return dict(id='Flow', args=[3, 1], returns=2, vectors=vectors)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base', type=Path, required=True)
    args = parser.parse_args()
    base = args.base.resolve()
    verify_inputs(base)
    source = base / 'native-vectors'
    source.mkdir(exist_ok=True)
    (source / 'main.act').write_text('MODULE ANALYSIS\nUSE EXECLISTS\nUSE RECORDPROBE\nPROC Main() RETURN\nENDMODULE\n')
    shutil.copyfile(base / 'exec/lib/exec/execlists.act', source / 'execlists.act')
    shutil.copyfile(Path(__file__).with_name('record_probe') / 'record_flow.act', source / 'recordprobe.act')
    artifacts = []
    all_cases = cases() + [flow_cases()]
    binary = base / 'compiler/target/debug/actionc-65816'
    for profile, (optimize, guards) in PROFILES.items():
        output = source / profile
        output.mkdir(exist_ok=True)
        layout = output / 'layout.json'
        save(layout, dict(code_origin=0x10000, data_origin=0x180000, stack_overflow=0x48000,
                         stack_checks=guards, nmi_extra_stack=0, imports=[]))
        command = [str(binary), '--layout', str(layout), '-o', str(output / 'image.json'),
                   *([] if optimize else ['--no-opt']), str(source / 'main.act')]
        subprocess.run(command, check=True)
        image = read(output / 'image.json')
        crlf = source / 'crlf'
        crlf.mkdir(exist_ok=True)
        for text in source.glob('*.act'):
            (crlf / text.name).write_bytes(text.read_text().replace('\r\n', '\n').replace('\n', '\r\n').encode())
        crlf_command = list(command)
        crlf_command[crlf_command.index('-o') + 1] = str(output / 'crlf-image.json')
        crlf_command[-1] = str(crlf / 'main.act')
        subprocess.run(crlf_command, check=True)
        if read(output / 'crlf-image.json') != image:
            raise ValueError('LF/CRLF compiler artifacts differ: ' + profile)
        guards = guard_ranges(image)
        for case in all_cases:
            prefix = 'M_RECORDPROBE_' if case['id'] == 'Flow' else 'M_EXECLISTS_'
            routine = next(r for r in image['routines'] if r['name'].startswith(prefix + case['id'].upper() + '_'))
            assert [a['size'] for a in routine['arguments']] == case['args']
            assert routine['result_bytes'] == case['returns']
            artifacts.append(dict(case=case['id'], compiler='actionc', mode='optimized' if optimize else 'raw',
                profile=profile, image=str(output / 'image.json'), commands=[command], entry=routine['address'],
                code_bytes=routine['size'], static_stack_check_bytes=sum(hi-lo for lo,hi in guards if routine['address'] <= lo < routine['address']+routine['size']),
                arguments=routine['arguments'], routines=image['routines'],
                code_ranges=[[r['address'], r['address'] + r['size']] for r in image['routines']],
                guard_ranges=guards, image_sha256=digest(output / 'image.json')))
    # The native runner keys modes by optimized/raw, so qualify profiles separately.
    for profile in PROFILES:
        save(source / f'{profile}.manifest.json', dict(schema=1, target='wdc-65816-native',
            observe_control_flow=False, cases=all_cases,
            guard_attribution='complete current native-v2 guard ranges; checked at instruction boundaries',
            artifacts=[a for a in artifacts if a['profile'] == profile]))
    save(source / 'sources.json', {p.name: digest(p) for p in source.glob('*.act')})
    print(sum(len(c['vectors']) for c in all_cases), 'vectors per profile; actual frozen ExecLists + independent mixed record flow')


if __name__ == '__main__':
    main()
