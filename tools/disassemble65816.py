#!/usr/bin/env python3
"""Disassemble native image segments emitted by the qualified scalar backend.

Each routine segment begins with M=X=0. Track explicit REP/SEP changes in the
emitter's linear instruction stream; reject unknown/truncated encodings.
Imported assembly is external to the image; use its assembler listing.
"""
import argparse
import json

IMPLIED = {0x18:'CLC',0x38:'SEC',0x1b:'TCS',0x3b:'TSC',0xaa:'TAX',0xa8:'TAY',0x98:'TYA',0x8a:'TXA',0xeb:'XBA',0x4b:'PHK',0x48:'PHA',0x3a:'DEC A',0x6b:'RTL',0xca:'DEX'}
DP = {0xa6:'LDX',0xa5:'LDA',0x85:'STA',0x65:'ADC',0xe5:'SBC',0xc5:'CMP',0x25:'AND',0x05:'ORA',0x45:'EOR',0x06:'ASL',0x26:'ROL',0x46:'LSR',0x66:'ROR'}
IMMEDIATE = {0xa9:'LDA',0x69:'ADC',0xe9:'SBC',0xc9:'CMP',0x29:'AND',0x49:'EOR',0xa0:'LDY',0xa2:'LDX'}
LONG = {0xaf:'LDA',0x8f:'STA',0x5c:'JML',0x22:'JSL'}
BRANCH = {0x10:'BPL',0x30:'BMI',0x90:'BCC',0xb0:'BCS',0xd0:'BNE',0xf0:'BEQ'}


def disassemble(image):
    if (image['format'], image['version'], image['abi']) != ('actionc-65816-image', 3, 'action65816.native.v1'):
        raise ValueError('unsupported native image')
    lines=[]
    for segment in image['segments']:
        if not segment['executable']:
            continue
        code=bytes(segment['bytes']);base=segment['address'];at=0;m8=x8=False
        while at<len(code):
            start=at;opcode=code[at];at+=1
            def operand(size):
                nonlocal at
                if at+size>len(code):raise ValueError(f'truncated instruction at {base+start:06X}')
                value=int.from_bytes(code[at:at+size],'little');at+=size;return value
            if opcode in IMPLIED:text=IMPLIED[opcode]
            elif opcode in (0xc2,0xe2):
                mask=operand(1);text=f'{"REP" if opcode==0xc2 else "SEP"} #${mask:02X}'
                if mask&0x20:m8=opcode==0xe2
                if mask&0x10:x8=opcode==0xe2
            elif opcode in IMMEDIATE:
                size=1 if (x8 if opcode in (0xa0,0xa2) else m8) else 2
                text=f'{IMMEDIATE[opcode]} #${operand(size):0{size*2}X}'
            elif opcode in DP:text=f'{DP[opcode]} ${operand(1):02X}'
            elif opcode in LONG:text=f'{LONG[opcode]} ${operand(3):06X}'
            elif opcode in (0xa3,0x83):text=f'{"LDA" if opcode==0xa3 else "STA"} ${operand(1):02X},S'
            elif opcode in (0xa7,0x87,0xb7,0x97):text=f'{"LDA" if opcode in (0xa7,0xb7) else "STA"} [${operand(1):02X}]'+(',Y' if opcode in (0xb7,0x97) else '')
            elif opcode in BRANCH:
                delta=operand(1);delta=delta if delta<128 else delta-256
                text=f'{BRANCH[opcode]} ${(base&0xff0000)|((base+at+delta)&0xffff):06X}'
            elif opcode==0x62:
                delta=operand(2);delta=delta if delta<32768 else delta-65536
                text=f'PER ${(base+at+delta)&0xffff:04X}'
            else:raise ValueError(f'unsupported opcode ${opcode:02X} at ${base+start:06X}')
            lines.append(f'{base+start:06X}  {code[start:at].hex(" ").upper():<11} {text}')
    return '\n'.join(lines)+'\n'


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('image');args=parser.parse_args()
    with open(args.image) as source:image=json.load(source)
    print(disassemble(image),end='')


if __name__=='__main__':main()
