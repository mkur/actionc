#ifndef TN_PLATFORM_H
#define TN_PLATFORM_H
#include "tn_target.h"
#include "tn_types.h"
#include <setjmp.h>
enum { TRUE=1,FALSE=0,EOL=155,ESC=27,NO_ITEM=0,
 SCREEN_COLS=40,SCREEN_ROWS=24,LAST_SCREEN_ROW=23,SCREEN_BYTES=960,
 WINDOW_ARENA=0x2000,WINDOW_ARENA_BYTES=0x800,
 CIO_OPEN=3,CIO_GET_RECORD=5,CIO_GET_CHARS=7,CIO_PUT_CHARS=11,CIO_CLOSE=12,
 OPEN_READ=4,OPEN_DIRECTORY=6,OPEN_WRITE=8,IO_SUCCESS=1,IO_EOF=136,
 CLEAR=0,FILL=0x40,INVERT=0x80 };
#define g_i MEM(0xE0)
#define g_j MEM(0xE1)
#define r0 MEM(0x70)
#define kx MEM(0xFF)
#define ky MEM(0xFE)
#define zx HW(0x5A)
#define zy HW(0x5B)
#define zdx HW(0x5C)
#define zdy HW(0x5D)
#define screen (*(Bytes*)0xE6)
#define allocp WORD(0xE8)
extern u8 Ioerr;
extern void (*ErrorHandler)(void);
extern jmp_buf tn_recovery;
extern volatile u8 tn_recover_pending;
extern u8 quickmul[48];
void tn_recover(void);
void tn_terminal(u16 vector);
void MovePage(Bytes dst,Bytes src,u8 len);
u16 Alloc(u16 n);
void Free(u16 address);
void Push(u16 data);
u16 Pull(void);
void CalcAdr(void);
void Position(u8 x,u8 y);
void Relpos(u8 x,u8 y);
u8 Internal(u8 ch);
u8 Ascii(u8 ch);
u8 Getchar(void);
u8 GetAnyKey(void);
void Store(u16 address,u8 value);
u8 Value(u16 address);
u8 _Cio(u8 channel,u8 value);
void _StrNam(u8 channel,Bytes name);
u8 GetD(u8 channel);
void PutD(u8 channel,u8 ch);
void Open(u8 channel,Bytes name,u8 aux1,u8 aux2);
void Close(u8 channel);
void Input(u8 channel,Bytes buffer,u8 len);
void Bget(u8 channel,Bytes buffer,u16 len);
void Bput(u8 channel,Bytes buffer,u16 len);
void Xio(u8 channel,u8 command,u8 aux1,u8 aux2,Bytes name);
#pragma compile("tn_platform.c")
#endif
