/* Source: modern/LIB.ACT machine/OS/arena routines and TN.ACT GetAnyKey,
   Store and Value. Inline assembly keeps inherited algorithms; parameter
   loads and accu stores adapt Oscar64's ABI, not Action!'s $A0 convention. */
#include "tn_platform.h"
u8 Ioerr;
void (*ErrorHandler)(void);
jmp_buf tn_recovery;
volatile u8 tn_recover_pending;
static u16 tn_handoff;
u8 quickmul[48];
static const u8 internalMap[4]={0x40,0,0x20,0x60};
static const u8 asciiMap[4]={0x20,0x40,0,0x60};
static u8 fnam[40];
/* The pinned compiler treats setjmp's ordinary return as constant zero.
   Resume through an explicit volatile flag, not its folded return value. */
void tn_recover(void) { tn_recover_pending=1; longjmp(tn_recovery,1); }
__noinline void tn_terminal(u16 vector) {
    tn_handoff=vector;
    __asm volatile { jmp (tn_handoff) }
}
void MovePage(Bytes dst,Bytes src,u8 len) {
    __asm {
        ldy len
        dey
    .copy:
        lda (src),y
        sta (dst),y
        dey
        cpy #0xFF
        bne .copy
    }
}
u16 Alloc(u16 n) { allocp=(u16)(allocp+n); return (u16)(allocp-n); }
void Free(u16 address) { allocp=address; }
void Push(u16 data) { WORD(allocp)=data; allocp=(u16)(allocp+2); }
u16 Pull(void) { allocp=(u16)(allocp-2); return WORD(allocp); }
void CalcAdr(void) {
    __asm {
        ldx 0xFE
        lda 0x58
        clc
        adc quickmul,x
        sta 0xE6
        lda 0x59
        adc quickmul+24,x
        sta 0xE7
        lda 0xE6
        adc 0xFF
        sta 0xE6
        bcc .done
        inc 0xE7
    .done:
    }
}
void Position(u8 x,u8 y) { kx=x; ky=y; }
void Relpos(u8 x,u8 y) {
    u16 sum=(u16)x+zx;
    kx=(u8)sum; ky=(u8)((u16)y+zy+(sum>>8));
}
u8 Internal(u8 ch) {
    return __asm {
        lda ch
        tay
        rol
        rol
        rol
        rol
        and #3
        tax
        tya
        and #0x9F
        ora internalMap,x
        sta accu
    };
}
u8 Ascii(u8 ch) {
    return __asm {
        lda ch
        tay
        rol
        rol
        rol
        rol
        and #3
        tax
        tya
        and #0x9F
        ora asciiMap,x
        sta accu
    };
}
u8 Getchar(void) {
    return __asm volatile {
        lda #0xFF
        sta 0x02FC
        lda #>.gotchar-1
        pha
        lda #<.gotchar-1
        pha
        lda 0xE425
        pha
        lda 0xE424
        pha
        rts
    .gotchar:
        sta accu
    };
}
u8 GetAnyKey(void) { while (!(HW(0xD20F)&4)) {} return Getchar(); }
void Store(u16 address,u8 value) { MEM(address)=value; }
u8 Value(u16 address) { return MEM(address); }
/* _Cio's original negative-status jump is expressed through the explicit
   recovery boundary. EOF invokes a returning handler, as in NavError. */
u8 _Cio(u8 channel,u8 value) {
    u8 result;
    result=__asm {
        ldx channel
        lda value
        jsr 0xE456
        sty Ioerr
        sta accu
    };
    if (Ioerr>=128 && ErrorHandler) ErrorHandler();
    return result;
}
void _StrNam(u8 channel,Bytes name) {
    u8 n=name[0],i=0;
    do { fnam[i]=name[(u8)(i+1)]; ++i; } while(i!=n);
    fnam[i]=EOL;
    WORD(0x344+(u16)(channel<<4))=(u16)fnam;
}
u8 GetD(u8 channel) {
    u16 base=0x340+(u16)(channel<<4);
    WORD(base+8)=0; MEM(base+2)=CIO_GET_CHARS;
    return _Cio((u8)(channel<<4),0);
}
void PutD(u8 channel,u8 ch) {
    u16 base=0x340+(u16)(channel<<4);
    WORD(base+8)=0; MEM(base+2)=CIO_PUT_CHARS;
    _Cio((u8)(channel<<4),ch);
}
void Open(u8 channel,Bytes name,u8 aux1,u8 aux2) {
    u16 base=0x340+(u16)(channel<<4);
    _StrNam(channel,name); MEM(base+10)=aux1; MEM(base+11)=aux2;
    MEM(base+2)=CIO_OPEN; _Cio((u8)(channel<<4),CIO_OPEN);
}
void Close(u8 channel) {
    MEM(0x342+(u16)(channel<<4))=CIO_CLOSE;
    _Cio((u8)(channel<<4),CIO_CLOSE);
}
void Input(u8 channel,Bytes buffer,u8 len) {
    u16 base=0x340+(u16)(channel<<4);
    WORD(base+4)=(u16)buffer; WORD(base+8)=len;
    MEM(base+2)=CIO_GET_RECORD; _Cio((u8)(channel<<4),CIO_GET_RECORD);
}
void Bget(u8 channel,Bytes buffer,u16 len) {
    u16 base=0x340+(u16)(channel<<4);
    WORD(base+4)=(u16)buffer; WORD(base+8)=len;
    MEM(base+2)=CIO_GET_CHARS; _Cio((u8)(channel<<4),CIO_GET_CHARS);
}
void Bput(u8 channel,Bytes buffer,u16 len) {
    u16 base=0x340+(u16)(channel<<4);
    WORD(base+4)=(u16)buffer; WORD(base+8)=len;
    MEM(base+2)=CIO_PUT_CHARS; _Cio((u8)(channel<<4),CIO_PUT_CHARS);
}
void Xio(u8 channel,u8 command,u8 aux1,u8 aux2,Bytes name) {
    u16 base=0x340+(u16)(channel<<4);
    MEM(base+2)=command; MEM(base+10)=aux1; MEM(base+11)=aux2;
    _StrNam(channel,name); _Cio((u8)(channel<<4),0);
}
