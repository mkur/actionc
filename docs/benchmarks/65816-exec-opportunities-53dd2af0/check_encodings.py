import json,re,subprocess
from pathlib import Path
out=Path('target/exec-ranking-53dd2af0')
rows=json.loads((out/'indexed_address_bounded.json').read_text())
asm=['.setcpu "65816"','.segment "CODE"','.i16']
for n,s in enumerate(rows):
    op=s['description'];stride=int(re.search(r'stride: ByteSize\((\d+)\)',op)[1]);disp=int(re.search(r'displacement: ByteOffset\((\d+)\)',op)[1]);asm+=['.a16',f'case{n}:']
    if s['subset']=='constant index':
        value=int(re.search(r'index: Some\(Mir65816Index \{ value: U\d+\((\d+)\)',op)[1])*stride+disp
        if not value:asm+=['rep #$20','.a16','lda $20,s','sta $30,s','lda $21,s','sta $31,s','sep #$20','.a8']
        else:asm+=['rep #$20','.a16','lda $20,s','clc',f'adc #{value}','sta $30,s','sep #$20','.a8','lda $22,s','adc #0','sta $32,s']
    else:
        asm+=['sep #$20','.a8','lda $10,s','rep #$20','.a16','and #$00ff']
        if stride.bit_count()>1:asm+=['sta $14']
        for bit in bin(stride)[3:]:
            asm+=['asl a']
            if bit=='1':asm+=['clc','adc $14']
        if disp:asm+=['clc',f'adc #{disp}']
        asm+=['clc','adc $20,s','sta $30,s','sep #$20','.a8','lda $22,s','adc #0','sta $32,s']
    asm+=[f'.assert *-case{n}={s["replacement"]}, error, "index model length"']
for n,operation in enumerate(['adc','sbc','and','ora','eor']):
    for j,right in enumerate(['#$03','$12,s','$12']):
        asm+=['.a8',f'alu{n}_{j}:','lda $10,s',f'{operation} {right}','sta $14,s',f'.assert *-alu{n}_{j}=6, error, "BYTE ALU length"']
(out/'model-encodings.s').write_text('\n'.join(asm)+'\n')
subprocess.run(['ca65',str(out/'model-encodings.s'),'-o',str(out/'model-encodings.o')],check=True)
print(f'Assembled {len(rows)} indexed-address replacements and 15 BYTE arithmetic operand forms; all length assertions passed. No execution qualification.')
