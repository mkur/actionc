/* Source: LIB.ACT Block/GetImage/PutImage. Existing machine loops, with
   compiler-owned __tmp instead of Action! argument/result scratch. CalcAdr's
   audited body preserves this work area. Widths and backwards traversal stay. */
#include "tn.h"
void Block(u8 mode,u8 dx,u8 dy) {
    __asm {
        jsr CalcAdr
        ldx dy
        lda mode
        cmp #0x80
        beq .invert
        cmp #0x40
        beq .fillrow
        lda #0
        sta 0x70
    .fillrow:
        ldy dx
        lda 0x70
    .fillcol:
        sta (0xE6),y
        dey
        bpl .fillcol
        clc
        lda 0xE6
        adc #40
        sta 0xE6
        bcc .fillnext
        inc 0xE7
    .fillnext:
        dex
        bpl .fillrow
        jmp .done
    .invert:
        ldy dx
    .invertcol:
        lda (0xE6),y
        eor #0x80
        sta (0xE6),y
        dey
        bpl .invertcol
        clc
        lda 0xE6
        adc #40
        sta 0xE6
        bcc .invertnext
        inc 0xE7
    .invertnext:
        dex
        bpl .invert
    .done:
    }
}
void GetImage(Bytes address,u8 dx,u8 dy) {
    __asm {
        lda address
        sta __tmp
        lda address+1
        sta __tmp+1
        lda dx
        sta __tmp+2
        clc
        adc #1
        sta __tmp+3
        jsr CalcAdr
        ldx dy
    .row:
        ldy __tmp+2
    .col:
        lda (0xE6),y
        sta (__tmp),y
        dey
        bpl .col
        clc
        lda 0xE6
        adc #40
        sta 0xE6
        bcc .screenok
        inc 0xE7
        clc
    .screenok:
        lda __tmp
        adc __tmp+3
        sta __tmp
        bcc .bufferok
        inc __tmp+1
    .bufferok:
        dex
        bpl .row
    }
}
void PutImage(Bytes address,u8 dx,u8 dy) {
    __asm {
        lda address
        sta __tmp
        lda address+1
        sta __tmp+1
        lda dx
        sta __tmp+2
        clc
        adc #1
        sta __tmp+3
        jsr CalcAdr
        ldx dy
    .row:
        ldy __tmp+2
    .col:
        lda (__tmp),y
        sta (0xE6),y
        dey
        bpl .col
        clc
        lda 0xE6
        adc #40
        sta 0xE6
        bcc .screenok
        inc 0xE7
        clc
    .screenok:
        lda __tmp
        adc __tmp+3
        sta __tmp
        bcc .bufferok
        inc __tmp+1
    .bufferok:
        dex
        bpl .row
    }
}
