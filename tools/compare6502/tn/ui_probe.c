#include "tn.h"
#include "tn_menus.h"
__noinline void ui_done(void) {__asm volatile {nop}}
int main(void) {
    Init(); WORD(0x58)=0x7800;
    do {
    Position(3,5); zx=3;zy=5;zdx=12;zdy=7;
    switch(HW(0x0600)) {
    case 1:HW(0x0601)=PopUp(menu_density,HW(0x0602),2);break;
    case 2:HW(0x0601)=PopUp(menu_yescancel,HW(0x0602),3);break;
    case 3:HW(0x0601)=InputLine((Bytes)0x1000,12,0);break;
    case 4:Window(10,7,17,9);Window(12,9,11,4);CloseAll();break;
    }
    WORD(0x0604)=allocp;HW(0x0606)=openwindows;
    for(u8 i=0;i<4;++i) HW(0x0608+i)=MEM(0x5A+i);
    WORD(0x060C)=(u16)menu_density;
    ui_done();}while(1);return 0;
}
