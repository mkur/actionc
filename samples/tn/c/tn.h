#ifndef TN_H
#define TN_H
#include "tn_platform.h"
enum { VISIBLE_FILES=16,COPY_SLOTS=16,LAST_VISIBLE_FILE=15,
 PATH_LEVELS=5,PATH_NAME_BYTES=9,MAX_PATH_DEPTH=4,PATH_SECTOR_BYTES=10,
 PATH_BUFFER_BYTES=47,DIR_SLOTS=65,DIR_ENTRY_BYTES=18,DIR_READ_BYTES=19,
 DIR_BUFFER_BYTES=1171,TAG_MASK=0x7F,PROTECTED_MARKER=0x0A,DIRECTORY_MARKER=0x1A,
 ROOT_DIRECTORY=0x0169,CIO_RENAME=32,CIO_DELETE=33,CIO_MKDIR=34,
 CIO_FORMAT=254,CIO_LOCK=35,CIO_UNLOCK=36,CIO_LOAD=39,CIO_CHDIR=41 };
#define g_dos HW(0x700)
#define g_dostst HW(0x76F)
#define g_ramdisk HW(0x70A)
#define g_drive HW(0x70B)
#define drvdef_1 ((Bytes)0x7CB)
#define dktype_1 ((Bytes)0x7C3)
#ifdef TN_DIAGNOSTIC
#define TN_API __export __noinline
#else
#define TN_API
#endif
#ifdef TN_WITH_UI
#define TN_UI_API TN_API
#else
#define TN_UI_API
#endif
extern Bytes buffer,s;
extern Words v,dirsectors,currentDir;
extern u16 dirnames[PATH_LEVELS];
extern u8 fname[15];
#define fnamelen fname[0]
extern u16 linebuf,dummy;
extern PanelState active;
extern u8 copyflag,validdir,ans,h,winnum;
TN_API void Strcpy(Bytes dst,Bytes src);
TN_API void Strcat(Bytes dst,Bytes src);
TN_API u8 Instr(Bytes text,u8 ch);
TN_API void PrintFname(Bytes text);
TN_API u8 Fnamecmp(Bytes left,Bytes right);
TN_API void Sort(void);
TN_API void Convert(Bytes text);
TN_API void Putchar(u8 ch);
TN_API void Print(Bytes text);
TN_API void PrintE(Bytes text);
void Block(u8 mode,u8 dx,u8 dy);
void GetImage(Bytes address,u8 dx,u8 dy);
void PutImage(Bytes address,u8 dx,u8 dy);
void Init(void);
extern u8 openwindows,frame[10];
TN_UI_API void SavePos(void);
TN_UI_API void RestorePos(void);
TN_UI_API void Window(u8 x,u8 y,u8 dx,u8 dy);
TN_UI_API void CloseWindow(void);
TN_UI_API void CloseAll(void);
TN_UI_API u8 Range(Bytes text,u8 max,Bytes choice);
TN_UI_API u8 InputLine(Bytes text,u8 max,u8 position);
TN_UI_API u8 Key(Bytes menu);
TN_UI_API Bytes Next(Bytes menu);
TN_UI_API u8 Ord(Bytes menu,Bytes item);
TN_UI_API u8 Items(Bytes menu);
TN_UI_API Bytes FindItem(Bytes menu,u8 ch);
TN_UI_API void DrawMenu(Bytes menu);
TN_UI_API Bytes MoveMenuBar(Bytes menu,u8 choice);
TN_UI_API u8 PopUp(Bytes menu,u8 choice,u8 spacing);
#ifdef TN_WITH_PANELS
TN_API void Path(void);
TN_API void Draw(u8 count);
TN_API void Inv(u8 count);
TN_API void UpdDis(void);
TN_API void DrawWinFrame(void);
TN_API u8 TaggedFiles(void);
TN_API u8 IsTagged(u8 i);
TN_API u8 IsProtected(u8 i);
TN_API u8 IsDirectory(u8 i);
TN_API void Tag(u8 i);
TN_API void TagAll(void);
TN_API u8 FindNext(u8 start);
TN_API void SetWin(u8 panel,u8 drive);
TN_API void SwapWin(void);
TN_API void Dir(void);
TN_API void InitPanels(void);
#pragma compile("tn_graphics.c")
#pragma compile("tn_panels.c")
#pragma compile("tn_lib.c")
#endif
enum { CMD_NONE=0,CMD_VIEW=1,CMD_ATTRIB=2,CMD_DELETE=3,CMD_RENAME=4,
 CMD_TAG_ALL=5,CMD_COPY=6,CMD_MKDIR=7,CMD_FORMAT=8,
#ifdef TN_DEBUG
 CMD_DBG=9,CMD_QUIT=10
#else
 CMD_QUIT=9
#endif
};
void PrintB(u8 value);
void GoTo(u8 index);
void Xloop(u8 command);
void View(void);void Attrib(void);void Delete(void);void Rename(void);
void Copy(void);void MkDir(void);void Format(void);void Quit(void);
void NewDrive(void);void SwapScr(void);void Dbg(void);
#ifdef TN_WITH_DISPATCH
TN_API void MakeJmp(u8 command);
TN_API void Handle(void);
#pragma compile("tn_dispatch.c")
#endif
#pragma compile("tn_core.c")
#endif
