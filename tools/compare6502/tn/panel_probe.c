#include "tn.h"
__noinline void panel_ready(void) {__asm volatile {nop}}
static void publish(void) {
    WORD(0x0602)=(u16)&active;WORD(0x0604)=(u16)v;WORD(0x0606)=(u16)dirsectors;
    WORD(0x0608)=(u16)dirnames;WORD(0x060A)=*currentDir;
    HW(0x060C)=g_drive;HW(0x060D)=winnum;WORD(0x060E)=allocp;
}
int main(void) {
    Init();InitPanels();
    do {
        publish();panel_ready();
        switch(HW(0x0600)) {
        case 1:SwapWin();break;
        case 2:SetWin(winnum,0);break;
        case 3:Tag(HW(0x0601));break;
        case 4:TagAll();break;
        case 5:Dir();break;
        case 6:SetWin(winnum,HW(0x0601));break;
        case 7:InitPanels();break;
        }
    } while(1);
    return 0;
}
