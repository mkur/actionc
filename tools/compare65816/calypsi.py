"""Calypsi linker inventory and relocated listing for the Exec list probe."""
import re


LINKER_RULES = """(define memories
 '((memory Code (address (#x10000 . #x1ffff)) (section farcode cfar chuge))
   (memory DirectPage (address (#x2000 . #x20ff)) (section registers))
   (base-address _DirectPageStart DirectPage 0)))
"""


def inventory(directory):
    mapping = (directory/'code.map').read_text().replace('\r\n', '\n')
    entries = re.findall(
        r"^(\S+) in section '(\w+)'\s+placed at address ([0-9a-f]+)-([0-9a-f]+) of size ([0-9a-f]+)\n"
        r"\([^\n]+ unit 0 section index (\d+)\)", mapping, re.M)
    sections = {}
    for name, kind, lo, hi, size, index in entries:
        lo, hi, size = (int(x, 16) for x in (lo, hi, size))
        assert hi+1-lo == size
        sections[int(index)] = dict(name=name, kind=kind, address=lo, size=size)
    code = (directory/'code.raw').read_bytes()
    assert sum(s['size'] for s in sections.values()) == len(code)
    layout = next(s for s in sections.values() if s['name']=='execlists_layout')
    start = layout['address']-0x10000
    expected = [3, 6, 9, 11, 11, 3, 6, 7, 8, 3, 6, 9, 10]
    assert code[start:start+layout['size']] == b''.join(x.to_bytes(2, 'little') for x in expected)
    # Assembler section numbering starts at 2. Every .section starts a fragment;
    # instruction lengths come from its listing, bytes from the linked image.
    index = 1
    covered = set()
    lines = []
    for line in (directory/'code.lst').read_text().splitlines():
        if re.match(r'^\d+\s+\.section\s', line):
            index += 1
            lines.append('\n; '+sections[index]['name'])
        match = re.match(r'^\d+\s+([0-9a-f]{6}) ([0-9a-f.]+)\s*(.*)$', line)
        if not match:
            continue
        offset, encoded, source = match.groups()
        address = sections[index]['address']+int(offset, 16)
        at, size = address-0x10000, len(encoded)//2
        assert len(encoded)%2 == 0 and 0 <= at < at+size <= len(code)
        assert covered.isdisjoint(range(at, at+size))
        covered.update(range(at, at+size))
        lines.append(f'{address:06X}  {code[at:at+size].hex(" ").upper():<11} {source}')
    assert covered == set(range(len(code)))
    (directory/'code.linked.lst').write_text('\n'.join(lines)+'\n')
    (directory/'code.bin').write_bytes(code)
    return [s for s in sections.values() if s['kind']=='farcode']
