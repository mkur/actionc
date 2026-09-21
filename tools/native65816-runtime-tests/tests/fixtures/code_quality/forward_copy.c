/* Exact unsigned widths; compile with -mhuge -ptr24. */
typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned long u32;
typedef char check_u16[sizeof(u16)==2 ? 1 : -1];
typedef char check_u32[sizeof(u32)==4 ? 1 : -1];
typedef char check_ptr[sizeof(u8 *)==3 ? 1 : -1];
void Work(u8 *dst,const u8 *src,u16 n) { while(n!=0) { *dst++=*src++; --n; } }
