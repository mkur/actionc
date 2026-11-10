#include "tn_platform.h"
static volatile u8 attempts;
static void eof_handler(void) { if(Ioerr!=IO_EOF) tn_recover(); }
static __noinline void nested(u8 depth) {
    volatile u8 state[12];
    state[0]=depth;
    if(depth) nested((u8)(depth-1)); else Close(1);
    HW(0x060E)=state[0]+1; /* every failing nested call must be abandoned */
}
void platform_done(void) { __asm { .halt: jmp .halt } }
int main(void) {
    ErrorHandler=tn_recover;
    tn_recover_pending=0;
    setjmp(tn_recovery);
    if(tn_recover_pending) { ++attempts; tn_recover_pending=0; }
    __asm {
        tsx
        stx 0x0618
        lda __sp
        sta 0x0619
        lda __sp+1
        sta 0x061A
    }
    if(!attempts) {
        HW(0x061B)=HW(0x0618); WORD(0x061C)=WORD(0x0619);
    }
    if(attempts<8) nested(4);
    HW(0x0601)=attempts;
    for(u16 n=0;n<256;++n) {
        MEM(0x0800+n)=Internal((u8)n);
        MEM(0x0900+n)=Ascii((u8)n);
    }
    MovePage((Bytes)0x1300,(Bytes)0x10FE,HW(0x0610));
    allocp=0x2000; Push(0x1234); Push(0xFE01);
    WORD(0x0602)=Pull(); WORD(0x0604)=Pull(); WORD(0x0606)=allocp;
    for(u8 row=0;row<24;++row) { quickmul[row]=(u8)(row*40); quickmul[row+24]=(u8)((row*40)>>8); }
    WORD(0x58)=0x70FE; Position(2,3); CalcAdr(); WORD(0x060A)=(u16)screen;
    zx=250; zy=4; Relpos(10,2); MEM(0x060C)=kx; MEM(0x060D)=ky;
    HW(0x060F)=Getchar();
    ErrorHandler=0;
    Open(2,(Bytes)0x1400,4,0);
    Input(2,(Bytes)0x1500,19);
    Bget(2,(Bytes)0x1600,0x123);
    Bput(2,(Bytes)0x1600,0x123);
    Xio(2,33,4,0,(Bytes)0x1400);
    ErrorHandler=eof_handler;
    HW(0x0608)=GetD(0); HW(0x0609)=Ioerr;
    PutD(0,0x5C); Close(0);
    HW(0x0600)=0xA5;
    tn_terminal(WORD(0x0620));
    return 0;
}
