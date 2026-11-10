/* Source: LIB.ACT shared persistent window state and Init. frame's following
   byte is spaces in the original layout; Init reads that byte at index 9. */
#include "tn.h"
u8 openwindows;
u8 frame[10]={8,0x11,0x12,0x05,0x7C,0x7C,0x1A,0x12,0x03,0};
void Init(void) {
    u16 h=0;
    for(g_i=0;g_i<=LAST_SCREEN_ROW;++g_i) {quickmul[g_i]=(u8)h;quickmul[g_i+24]=(u8)(h>>8);h+=SCREEN_COLS;}
    for(g_i=0;g_i<=8;++g_i) frame[g_i]=Internal(frame[g_i+1]);
    allocp=WINDOW_ARENA;openwindows=0;
}
