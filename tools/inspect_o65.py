#!/usr/bin/env python3
"""Independent standard-o65 decoder used by conformance tests (no compiler imports)."""
import argparse
import hashlib
import json
from pathlib import Path


def inspect(data):
    cursor = 0

    def take(size):
        nonlocal cursor
        if size < 0 or cursor + size > len(data):
            raise ValueError('truncated o65')
        value = data[cursor:cursor + size]
        cursor += size
        return value

    def number(size):
        return int.from_bytes(take(size), 'little')

    def name():
        nonlocal cursor
        end = data.find(b'\0', cursor, min(len(data), cursor + 4097))
        if end < 0:
            raise ValueError('unterminated symbol')
        value = take(end - cursor).decode('ascii')
        take(1)
        return value

    if take(6) != b'\x01\0o65\0':
        raise ValueError('bad magic')
    mode = number(2)
    width = 4 if mode & 0x2000 else 2
    if not mode & 0x8000 or mode & 0x4400:
        raise ValueError('unsupported CPU/paged/chained mode')
    sections = []
    for _ in range(4):
        sections.append({'base': number(width), 'size': number(width)})
    stack = number(width)
    while True:
        size = number(1)
        if not size:
            break
        if size < 2:
            raise ValueError('invalid option')
        take(size - 1)
    payloads = [take(sections[i]['size']) for i in (0, 1)]
    imports = [name() for _ in range(number(width))]
    relocations = []
    for section, payload in zip((2, 3), payloads):
        at = -1
        end = 0
        while True:
            delta = number(1)
            if not delta:
                break
            at += 254 if delta == 255 else delta
            if delta == 255:
                continue
            tag = number(1)
            kind, target = tag & 0xe0, tag & 0x1f
            if kind not in (0x20, 0x40, 0x80, 0xa0, 0xc0) or target not in (0, 2, 3, 4, 5):
                raise ValueError('invalid relocation')
            imported = number(width) if target == 0 else None
            size = {0x80: 2, 0xc0: 3}.get(kind, 1)
            if at < end or at + size > len(payload):
                raise ValueError('relocation bounds')
            end = at + size
            value = int.from_bytes(payload[at:end], 'little')
            if kind == 0x40:
                value = (value << 8) | number(1)
            if kind == 0xa0:
                value = (value << 16) | number(2)
            if imported is not None and imported >= len(imports):
                raise ValueError('import index')
            relocations.append({'section': section, 'offset': at, 'kind': kind,
                                'target': target, 'import': imported, 'value': value})
    exports = []
    for _ in range(number(width)):
        exports.append({'name': name(), 'segment': number(1), 'value': number(width)})
    if cursor != len(data):
        raise ValueError('trailing bytes')
    return {'mode': mode, 'width': width, 'sections': sections, 'stack': stack,
            'imports': imports, 'relocations': relocations, 'exports': exports,
            'text_sha256': hashlib.sha256(payloads[0]).hexdigest(),
            'data_sha256': hashlib.sha256(payloads[1]).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('file', type=Path)
    args = parser.parse_args()
    print(json.dumps(inspect(args.file.read_bytes()), sort_keys=True))


if __name__ == '__main__':
    main()
