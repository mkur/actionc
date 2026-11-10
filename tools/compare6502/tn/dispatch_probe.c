/* Command interception is confined to this diagnostic input. Production
   dispatch resolves to the real command implementations in tn_commands.c. */
#include "tn.h"
#define EVENT(n) (++HW(0x0B00+(n)))
void View(void){EVENT(CMD_VIEW);}void Attrib(void){EVENT(CMD_ATTRIB);}
void Delete(void){EVENT(CMD_DELETE);}void Rename(void){EVENT(CMD_RENAME);}
void Copy(void){EVENT(CMD_COPY);}void MkDir(void){EVENT(CMD_MKDIR);}
void Format(void){EVENT(CMD_FORMAT);}void Quit(void){EVENT(CMD_QUIT);}
void Dbg(void){EVENT(9);}void NewDrive(void){ans=ESC;}
void SwapScr(void){}
void GoTo(u8 index){active.selected=index;active.row=index>15?15:index;UpdDis();}
int main(void) {
    Init();WORD(0x58)=0x7800;g_dos='M';currentDir=(Words)0x7B8;g_drive=2;
    active.files=HW(0x0600);v=(Words)0x1000;winnum=0;zx=1;zy=4;
    for(u8 i=0;i<65;++i) v[i]=0x1200+(u16)i*18;
    for(u16 i=0;i<1171;++i) MEM(0x1200+i)=0;
    Handle();return 0;
}
