#include "tn.h"
/* A test-only callable copy of the source SetWin conversion region. The full
   SetWin port keeps these statements in their original enclosing routine. */
__noinline void SetWinConversion(Bytes row) {
    s=row;
    if(s[1]==32) s[1]=Internal(s[0]); else s[1]=Internal(s[1]);
    s[0]=0;
    for(g_i=2;g_i<=9;++g_i) s[g_i]=Internal(s[g_i]);
}
__noinline void kernel_begin(void) { __asm volatile { nop } }
void kernel_done(void) { __asm { .halt: jmp .halt } }
int main(void) {
    volatile u8 *box=(volatile u8*)0x0600;
    Bytes left=(Bytes)WORD(0x0602),right=(Bytes)WORD(0x0604);
    for(u8 n=0;n<24;++n) { quickmul[n]=(u8)(n*40);quickmul[n+24]=(u8)((n*40)>>8); }
    WORD(0x58)=0x7800;kx=0;ky=0;
    v=(Words)WORD(0x0606);active.files=box[8];
    kernel_begin();
    switch(box[0]) {
    case 1:Strcpy(left,right);break;
    case 2:Strcat(left,right);break;
    case 3:box[1]=Instr(left,box[9]);break;
    case 4:box[1]=Fnamecmp(left,right);break;
    case 5:Sort();break;
    case 6:Convert(left);for(u8 n=0;n<15;++n) HW(0x0A00+n)=fname[n];break;
    case 7:SetWinConversion(left);break;
    case 8:Print(left);break;
    case 9:PrintFname(left);break;
    }
    box[10]=g_i;box[11]=g_j;
    kernel_done();return 0;
}
