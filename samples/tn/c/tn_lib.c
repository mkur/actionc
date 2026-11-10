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

/* Source: LIB.ACT windows, editor and menu routines. */
#include "tn_text.h"
#define spaces frame[9]
void SavePos(void) {u16 address=Alloc(4);MovePage((Bytes)address,(Bytes)0x5A,4);}
void RestorePos(void) {allocp-=4;MovePage((Bytes)0x5A,(Bytes)allocp,4);}
void Window(u8 x,u8 y,u8 dx,u8 dy) {
    u16 buf;u8 h;
    SavePos();++openwindows;
    /* The pinned compiler's combined zero-page copy loop branches into its
       LDX operand. Empty barriers retain these source-ordered scalar stores
       without inserting target instructions or changing their values. */
    zx=x; __asm volatile {} zy=y; __asm volatile {}
    zdx=dx; __asm volatile {} zdy=dy;Relpos(0,0);
    buf=Alloc((u16)(zdx+1)*(u16)(zdy+1));Push(buf);GetImage((Bytes)buf,zdx,zdy);SavePos();
    Block(CLEAR,zdx,zdy);CalcAdr();*screen=frame[0];h=(u8)(SCREEN_COLS-zdx);
    for(g_i=2;g_i<=zdx;++g_i) {++screen;*screen=frame[1];}
    ++screen;*screen=frame[2];
    for(g_i=2;g_i<=zdy;++g_i) {screen+=h;*screen=frame[3];screen+=zdx;*screen=frame[4];}
    screen+=h;*screen=frame[5];++screen;
    for(g_i=2;g_i<=zdx;++g_i) {*screen=frame[6];++screen;}
    *screen=frame[7];++zx;++zy;--zdx;--zdy;Relpos(0,0);
}
void CloseWindow(void) {u16 buf;RestorePos();--openwindows;Relpos(0,0);buf=Pull();Free(buf);PutImage((Bytes)buf,zdx,zdy);RestorePos();}
void CloseAll(void) {while(openwindows) CloseWindow();}
u8 Range(Bytes text,u8 max,Bytes choice) {
    /* Source-visible scratch cells, distinct from Oscar's relocated SP/FP. */
    #define range_i MEM(0xA0)
    #define range_len MEM(0xA1)
    #define range_f MEM(0xA2)
    #define range_ch MEM(0xA3)
    range_len=text[0];
    do {
        range_ch=Getchar();range_f=TRUE;
        for(range_i=1;range_i<=range_len;range_i=(u8)(range_i+2)) {
            if(text[range_i]==range_ch) {if(*choice!=0) --*choice;else range_f=FALSE;}
            else if(text[(u8)(range_i+1)]==range_ch) {if(*choice<max) ++*choice;else range_f=FALSE;}
            if(range_i>=254) break;
        }
    } while(!range_f);
    return range_ch;
}
u8 InputLine(Bytes text,u8 max,u8 position) {
    u8 ch;
    SavePos();zx=kx;zy=ky;
    do {
        Relpos(position,0);Block(INVERT,0,0);ch=Getchar();Block(INVERT,0,0);
        if(ch==126) {if(position!=0){--position;Relpos(position,0);Putchar(32);}}
        else if(ch==156) {position=0;Relpos(0,0);Block(CLEAR,(u8)(max-1),0);}
        else if(ch==ESC || ch==EOL) break;
        else if(position<max) {text[(u8)(position+1)]=ch;Putchar(ch);++position;}
    } while(1);
    text[0]=position;RestorePos();if(ch==ESC || position==0) return 0;return 1;
}
u8 Key(Bytes menu) {menu+=menu[0]+3;return *menu;}
Bytes Next(Bytes menu) {return menu+menu[0]+5;}
u8 Ord(Bytes menu,Bytes item) {u8 c=0;while(*menu){if(menu==item) break;++c;menu=Next(menu);}return c;}
u8 Items(Bytes menu) {u8 c=0;while(*menu){++c;menu=Next(menu);}return c;}
Bytes FindItem(Bytes menu,u8 ch) {while(*menu){if(Key(menu)==ch) return menu;menu=Next(menu);}return (Bytes)NO_ITEM;}
void DrawMenu(Bytes menu) {zx=kx;zy=ky;while(*menu){Print(menu);kx=(u8)(kx+spaces);menu=Next(menu);}}
Bytes MoveMenuBar(Bytes menu,u8 c) {
    u8 p=0,a;
    while(c) {a=(u8)(menu[0]+spaces);p=(u8)(p+a);--c;menu=Next(menu);}
    Relpos(p,0);Block(INVERT,(u8)(menu[0]-1),0);return menu;
}
u8 PopUp(Bytes mp,u8 c,u8 spc) {
    static Bytes menu;
    u8 ch,d,limit;
    Push((u16)menu);SavePos();menu=mp;spaces=spc;limit=(u8)(Items(menu)-1);
    DrawMenu(menu);mp=MoveMenuBar(menu,c);
    do {
        d=c;ch=Range(text_bytes_49a12d9f,limit,&c);
        {Bytes item=FindItem(menu,ch);if(item!=(Bytes)NO_ITEM){mp=item;c=Ord(menu,mp);ch=EOL;}}
        if(c!=d){MoveMenuBar(menu,d);mp=MoveMenuBar(menu,c);}
    } while(ch!=ESC && ch!=EOL);
    RestorePos();menu=(Bytes)Pull();if(ch==EOL) return Key(mp);return ESC;
}
