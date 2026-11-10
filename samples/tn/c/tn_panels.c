/* Source: TN.ACT Path..DrawWinFrame, TaggedFiles..Dir and InitPanels. */
#include "tn.h"
#include "tn_text.h"
static PanelState *panelData,*panelDataOld;
static PanelState leftPanel,rightPanel;
static u8 lp_PathBuf[PATH_BUFFER_BYTES],rp_PathBuf[PATH_BUFFER_BYTES];
static Bytes pathBuf,dirBuf;
static u8 lp_DirBuf[DIR_BUFFER_BYTES],rp_DirBuf[DIR_BUFFER_BYTES];
static u16 lp_v[DIR_SLOTS],rp_v[DIR_SLOTS];
static u8 panel_init_times;
void Path(void) {
    Position(0,1);Block(CLEAR,39,0);Putchar('D');Putchar((u8)('0'|g_drive));
    for(g_i=0;g_i<=active.nestLevel;++g_i) {Print((Bytes)dirnames[g_i]);Putchar(':');}
}
void Draw(u8 cnt) {
    if((u8)(VISIBLE_FILES+cnt)>active.files) r0=active.files;else r0=(u8)(cnt+VISIBLE_FILES);
    Relpos(0,17);PutImage((Bytes)v[active.files],16,0);Relpos(0,0);
    for(g_j=(u8)(cnt+1);g_j<=r0;++g_j) {PutImage((Bytes)v[(u8)(g_j-1)],17,0);++ky;}
}
void Inv(u8 cnt) {Relpos(0,cnt);Block(INVERT,9,0);Relpos(11,cnt);Block(INVERT,2,0);Relpos(15,cnt);Block(INVERT,2,0);}
void UpdDis(void) {Draw((u8)(active.selected-active.row));Inv(active.row);}
void DrawWinFrame(void) {
    zx=(u8)(20*winnum);zy=3;Relpos(0,255);Print(text_d);Putchar((u8)('0'|g_drive));Print(text_bytes_3b5b5b6c);
    Relpos(7,255);Print((Bytes)dirnames[active.nestLevel]);Relpos(16,255);
    if(g_ramdisk==g_drive) Putchar('R');else {
        if(dktype_1[g_drive]&2) Putchar('D');else Putchar('S');
        if(drvdef_1[g_drive]&1) Putchar('=');else Putchar('-');
    }
    Relpos(0,255);Block(INVERT,19,0);Relpos(0,0);PrintE(text_bytes_d66c6879);
    for(g_i=0;g_i<=LAST_VISIBLE_FILE;++g_i) PrintE(text_bytes_2811c21d);
    PrintE(text_bytes_5ffc6671);PrintE(text_bytes_fbfd3b63);PrintE(text_bytes_73fce786);
    ++zx;++zy;
}
u8 TaggedFiles(void) {return active.taggedCount;}
u8 IsTagged(u8 i) {DirectoryRow *entry=(DirectoryRow*)v[i];return entry->tag;}
u8 IsProtected(u8 i) {DirectoryRow *entry=(DirectoryRow*)v[i];return entry->marker==PROTECTED_MARKER?1:0;}
u8 IsDirectory(u8 i) {DirectoryRow *entry=(DirectoryRow*)v[i];return entry->marker==DIRECTORY_MARKER?1:0;}
void Tag(u8 i) {
    u8 a=IsTagged(i);
    if(a) --active.taggedCount;else ++active.taggedCount;
    Store(v[i],a^TAG_MASK);i=TaggedFiles();
    if(i==active.files) active.tagMask=TAG_MASK;else if(i==0) active.tagMask=0;
}
void TagAll(void) {
    if(active.tagMask) {active.tagMask=0;active.taggedCount=0;}
    else {active.tagMask=TAG_MASK;active.taggedCount=active.files;}
    for(g_i=1;g_i<=active.files;++g_i) Store(v[(u8)(g_i-1)],active.tagMask);
    UpdDis();
}
u8 FindNext(u8 c) {
    for(g_j=c;g_j<=(u8)(active.files-1);++g_j) {if(IsTagged(g_j)) return g_j;if(g_j==255) break;}
    return 255;
}
void SetWin(u8 n,u8 drvnum) {
    u8 k;
    winnum=n;panelDataOld=panelData;
    if(winnum) {pathBuf=lp_PathBuf;panelData=&leftPanel;v=lp_v;dirBuf=lp_DirBuf;}
    else {pathBuf=rp_PathBuf;panelData=&rightPanel;v=rp_v;dirBuf=rp_DirBuf;}
    dirsectors=(Words)pathBuf;pathBuf+=PATH_SECTOR_BYTES;pathBuf[0]=0;dirnames[0]=(u16)pathBuf;++pathBuf;
    for(g_i=1;g_i<=MAX_PATH_DEPTH;++g_i) {dirnames[g_i]=(u16)pathBuf;pathBuf+=PATH_NAME_BYTES;}
    if(drvnum) {
        if(drvnum!=255) {g_drive=drvnum;active.nestLevel=0;*currentDir=ROOT_DIRECTORY;}
        Path();Close(1);Open(1,text_d_bee2597f,OPEN_DIRECTORY,0);
        s=dirBuf;g_i=0;
        do {Input(1,s,DIR_READ_BYTES);if(Ioerr!=IO_SUCCESS) break;v[g_i]=(u16)s;s+=DIR_ENTRY_BYTES;++g_i;} while(1);
        active.files=(u8)(g_i-1);Sort();
        for(g_j=1;g_j<=active.files;++g_j) {
            s=(Bytes)v[(u8)(g_j-1)];
            if(s[1]==32) s[1]=Internal(s[0]);else s[1]=Internal(s[1]);
            s[0]=0;for(g_i=2;g_i<=9;++g_i) s[g_i]=Internal(s[g_i]);
            if(s[17]==EOL) k=1;else k=0;
            for(g_i=0;g_i<=2;++g_i) {s[13-g_i]=Internal(s[12-g_i]);s[17-g_i]=Internal(s[(17-k)-g_i]);}
            s[10]=124;s[14]=124;
        }
        s=(Bytes)v[active.files];g_i=0;
        while(s[g_i]!=EOL && g_i<=17) {s[g_i]=Internal(s[g_i]);++g_i;}
        for(g_j=g_i;g_j<=17;++g_j) s[g_j]=0;
        DrawWinFrame();panelDataOld=panelData;active.tagMask=TRUE;active.selected=0;active.row=0;TagAll();Inv(0);
    }
    active.directory=*currentDir;active.drive=g_drive;
    MovePage((Bytes)panelDataOld,(Bytes)&active,sizeof(PanelState));
    MovePage((Bytes)&active,(Bytes)panelData,sizeof(PanelState));
    g_drive=active.drive;*currentDir=active.directory;
}
void SwapWin(void) {SetWin(winnum^1,0);zx=(u8)(winnum*20+1);}
void Dir(void) {SetWin(winnum,255);}
void InitPanels(void) {
    if(panel_init_times==0) {
        if(g_dos=='M') {if(g_dostst==0xA9) currentDir=(Words)0x7B8;else currentDir=(Words)0x7BB;}
        else currentDir=&dummy;
        SetWin(0,g_drive);++panel_init_times;
    }
    if(panel_init_times==1) {
        ++panel_init_times;
        if(g_ramdisk==255) SetWin(1,g_drive);else SetWin(1,g_ramdisk);
        SwapWin();Path();
    }
}
