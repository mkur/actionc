#!/usr/bin/env python3
"""Hand-specified application fixture, independent of the Rust writer/decoder."""
import argparse
import struct
from pathlib import Path


def u32(value):
    return struct.pack('<I', value)


def string(value):
    value = value.encode('utf-8')
    return u32(len(value)) + value


def contract(raw):
    return (string('action65816.native.v1') + u32(0 if raw else 1234)
            + u32(0) + b'\0' + u32(0 if raw else 1)
            + struct.pack('<HBBB', 0, 0, int(raw), 3))


def build():
    # RTL; no compiler generated instruction or compiler metadata is used.
    routine = (u32(7) + string('Main') + u32(0) + u32(1)
               + contract(False) + struct.pack('<HHI', 0, 0, 0))
    objects = b''
    for identity, name, section, size in [(8, 'pointers', 3, 10), (9, 'buffer', 4, 8)]:
        objects += (b'\0' + u32(identity) + string(name) + bytes([section])
                    + u32(0) + u32(size) + u32(1) + b'\1\0')
    # LONG+BSS+4 in four bytes; LOW/HIGH/BANK+BSS+7; LONG+text+0.
    records = [(0, 0xc0, 4, 4, 1), (4, 0x20, 4, 7, 0),
               (5, 0x40, 4, 7, 0), (6, 0xa0, 4, 7, 0), (7, 0xc0, 2, 0, 0)]
    proofs = b''.join(bytes([3]) + u32(offset) + bytes([kind, target])
                      + u32(0) + u32(value) + bytes([wide])
                      for offset, kind, target, value, wide in records)
    descriptor = (b'A8O1' + u32(0) + struct.pack('<HHBBHI', 1, 0, 3, 0, 0, 0)
                  + u32(1) + routine + u32(2) + objects + u32(1)
                  + string('__a816_stack_overflow_v1') + contract(True)
                  + u32(len(records)) + proofs)
    descriptor = descriptor[:4] + u32(len(descriptor)) + descriptor[8:]
    text = b'\x6b' + descriptor
    data = bytes([4, 0, 0, 0, 7, 0, 0, 0, 0, 0])
    header = b'\1\0o65\0' + struct.pack('<H9I', 0xa202, 0, len(text), 0, len(data), 0, 8, 0, 0, 0) + b'\0'
    imports = u32(1) + b'__a816_stack_overflow_v1\0'
    # Text has no relocations. Each delta is from the previous site, first -1.
    relocations = bytes([0, 1, 0xc4, 4, 0x24, 1, 0x44, 7,
                         1, 0xa4, 7, 0, 1, 0xc2, 0])
    exports = (u32(2) + b'__a816_entry_v1\0\2' + u32(0)
               + b'__a816_o65_profile_v1\0\2' + u32(1))
    return header + text + data + imports + relocations + exports


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    path = Path(__file__).with_name('reference.o65')
    if args.check:
        assert path.read_bytes() == build(), 'reference fixture differs'
    else:
        path.write_bytes(build())


if __name__ == '__main__':
    main()
