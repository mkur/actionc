#include "tn_target.h"
#include "tn_types.h"
static u8 calls;
static const u8 counted[] = {3, 0x41, 0x9B, 0xC2};
static __noinline u16 mix(u8 a, u16 b, u8 c) { ++calls; return (u16)(b + a + c); }
void probe_done(void) { __asm { .halt: jmp .halt } }
int main(void) {
    volatile u8 *out=(volatile u8*)0x0600;
    PanelState p;
    p.directory=0x1234; p.taggedCount=0x7F;
    out[1]=sizeof(PanelState); out[2]=sizeof(DirectoryRow);
    out[3]=((u8*)&p.directory)[0]; out[4]=((u8*)&p.directory)[1];
    WORD(0x0605)=mix(out[16],WORD(0x0612),out[17]);
    out[7]=calls; out[8]=counted[3]; out[9]=p.taggedCount;
    out[0]=0xA5;
    probe_done();
    return 0;
}
