#!/usr/bin/env python3
"""Generate paired, freestanding Action!/C kernels and independent test vectors."""
import argparse
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEST = ROOT / 'tools/native65816-runtime-tests/tests/fixtures/code_quality'
C_HEADER = '''/* Exact unsigned widths; compile with -mhuge -ptr24. */
typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned long u32;
typedef char check_u16[sizeof(u16)==2 ? 1 : -1];
typedef char check_u32[sizeof(u32)==4 ? 1 : -1];
typedef char check_ptr[sizeof(u8 *)==3 ? 1 : -1];
'''


def vector(args, result=None, memory=None, after=None):
    return dict(args=args, result=result, memory=memory or [], after=after or [])


def region(address, data):
    return dict(address=address, bytes=list(data))


def ptr(value):
    return list(value.to_bytes(3, 'little'))


def cases():
    result = []

    def add(name, args, returns, action, c, vectors, note=''):
        result.append(dict(id=name, args=args, returns=returns, action=action,
                           c=c, vectors=vectors, note=note))

    values = [0, 1, 13, 32767, 32768, 65535]
    pairs = [(0, 0), (1, 2), (13, 41), (65535, 1), (32768, 32767)]
    add('identity', [2], 2,
        'PUBLIC CARD FUNC Work(CARD x) RETURN(x)',
        'u16 Work(u16 x) { return x; }',
        [vector([x], x) for x in values])
    add('add', [2, 2], 2,
        'PUBLIC CARD FUNC Work(CARD x,y) RETURN(x+y)',
        'u16 Work(u16 x,u16 y) { return x+y; }',
        [vector([x, y], (x+y) & 65535) for x, y in pairs])
    expression = '+'.join(['x', *map(str, range(1, 17))])
    add('constant_chain', [2], 2,
        f'PUBLIC CARD FUNC Work(CARD x) RETURN({expression})',
        f'u16 Work(u16 x) {{ return {expression}; }}',
        [vector([x], (x+136) & 65535) for x in values])
    add('maximum', [2, 2], 2,
        'PUBLIC CARD FUNC Work(CARD x,y) IF x>y THEN RETURN(x) FI RETURN(y)',
        'u16 Work(u16 x,u16 y) { if(x>y) return x; return y; }',
        [vector([x, y], max(x, y)) for x, y in pairs])
    add('wide_shift', [4], 4,
        'PUBLIC LONGCARD FUNC Work(LONGCARD x) RETURN((x LSH 5) XOR (x RSH 3))',
        'u32 Work(u32 x) { return (x<<5) ^ (x>>3); }',
        [vector([x], ((x<<5) ^ (x>>3)) & 0xffffffff)
         for x in [0, 1, 0x12345678, 0x80000000, 0xffffffff]])
    add('loop_rotation', [2], 2,
        '''PUBLIC CARD FUNC Work(CARD x)
CARD a,b,c,i
a=x b=x+1
FOR i=0 TO 7 DO c=a a=b b=c+1 OD
RETURN(a+b)''',
        '''u16 Work(u16 x) {
 u16 a=x,b=x+1,c,i;
 for(i=0;i<8;++i) { c=a; a=b; b=c+1; }
 return a+b;
}''',
        [vector([x], (2*x+9) & 65535) for x in values])
    add('sum_loop', [2], 2,
        '''PUBLIC CARD FUNC Work(CARD n)
CARD total
total=0
WHILE n#0 DO total==+n n==-1 OD
RETURN(total)''',
        'u16 Work(u16 n) { u16 total=0; while(n!=0) { total+=n; --n; } return total; }',
        [vector([n], n*(n+1)//2) for n in [0, 1, 8, 13, 31]])
    add('recursive_sum', [2], 2,
        '''PUBLIC CARD FUNC Work(CARD n)
IF n=0 THEN RETURN(0) FI
RETURN(n+Work(n-1))''',
        'u16 Work(u16 n) { if(n==0) return 0; return n+Work(n-1); }',
        [vector([n], n*(n+1)//2) for n in [0, 1, 8, 13]])
    add('direct_calls', [2, 2], 2,
        '''CARD FUNC Helper(CARD x) RETURN(x+7)
PUBLIC CARD FUNC Work(CARD x,y) RETURN(Helper(x)+Helper(y))''',
        '''u16 Helper(u16 x) { return x+7; }
u16 Work(u16 x,u16 y) { return Helper(x)+Helper(y); }''',
        [vector([x, y], (x+y+14) & 65535) for x, y in pairs],
        'Both public C functions and all emitted Action functions are counted; no forced inlining policy.')
    data = [1, 2, 3, 250, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
    address = 0x12fffc
    add('byte_sum', [3, 2], 2,
        '''PUBLIC CARD FUNC Work(BYTE POINTER p CARD n)
CARD total
total=0
WHILE n#0 DO total==+CARD(p^) p==+1 n==-1 OD
RETURN(total)''',
        'u16 Work(const u8 *p,u16 n) { u16 total=0; while(n!=0) { total+=*p++; --n; } return total; }',
        [vector([address, n], sum(data[:n]), [region(address, data)]) for n in [0, 1, 4, 8, 16]],
        'The buffer crosses a 64 KiB bank; huge 24-bit C pointers match full native pointer progression here.')
    add('record_field', [3], 2,
        '''TYPE Pair=[CARD low,high]
PUBLIC CARD FUNC Work(Pair POINTER p) RETURN(p.high)''',
        '''struct Pair { u16 low,high; };
typedef char check_pair[sizeof(struct Pair)==4 ? 1 : -1];
u16 Work(const struct Pair *p) { return p->high; }''',
        [vector([0x12fffe], n,
                [region(0x12fffe, [7, 0, n & 255, n >> 8])]) for n in [0, 13, 65535]],
        'Both records have two adjacent 16-bit fields. The second field is in the next bank.')
    previous, item, following = 0x12fffc, 0x130040, 0x14fffe
    before = [region(previous, ptr(item)+ptr(0)),
              region(item, ptr(following)+ptr(previous)),
              region(following, ptr(0)+ptr(item))]
    after = [region(previous, ptr(following)+ptr(0)), before[1],
             region(following, ptr(0)+ptr(previous))]
    add('unlink', [3], 0,
        '''TYPE Node=[Node POINTER next Node POINTER previous]
PUBLIC PROC Work(Node POINTER item)
Node POINTER previous,following
previous=item.previous following=item.next
previous.next=following following.previous=previous
RETURN''',
        '''struct Node { struct Node *next,*previous; };
typedef char check_node[sizeof(struct Node)==6 ? 1 : -1];
void Work(struct Node *item) {
 struct Node *previous=item->previous,*following=item->next;
 previous->next=following; following->previous=previous;
}''',
        [vector([item], memory=before, after=after)],
        'The same six-byte node layout and full 24-bit pointers; two nodes cross banks.')
    copies = []
    for src, dst, count in [(0, 8, 8), (0, 4, 8), (4, 0, 8), (0, 0, 0)]:
        initial = list(range(1, 17))
        output = initial.copy()
        for i in range(count):
            output[dst+i] = output[src+i]
        base = 0x12fff8
        copies.append(vector([base+dst, base+src, count],
                             memory=[region(base, initial)], after=[region(base, output)]))
    add('forward_copy', [3, 3, 2], 0,
        '''PUBLIC PROC Work(BYTE POINTER dst,src CARD n)
WHILE n#0 DO dst^=src^ dst==+1 src==+1 n==-1 OD
RETURN''',
        'void Work(u8 *dst,const u8 *src,u16 n) { while(n!=0) { *dst++=*src++; --n; } }',
        copies,
        'Explicit forward-copy semantics, including both overlap directions; neither source uses restrict or memcpy.')
    return result


def files():
    manifest = []
    output = {}
    for case in cases():
        name = case['id']
        output[name+'.act'] = 'MODULE CORPUS\n'+case['action']+'\nPROC Main() RETURN\nENDMODULE\n'
        output[name+'.c'] = C_HEADER+case['c']+'\n'
        manifest.append({k: v for k, v in case.items() if k not in ('action', 'c')})
    output['cases.json'] = json.dumps(manifest, indent=2)+'\n'
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    DEST.mkdir(parents=True, exist_ok=True)
    for name, text in files().items():
        path = DEST / name
        if args.check:
            assert path.read_text().replace('\r\n', '\n') == text, path
        else:
            path.write_text(text)
    print(('Checked' if args.check else 'Generated'), len(cases()), 'parallel kernels.')


if __name__ == '__main__':
    main()
