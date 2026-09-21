/* Exact unsigned widths; compile with -mhuge -ptr24. */
typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned long u32;
typedef char check_u16[sizeof(u16)==2 ? 1 : -1];
typedef char check_u32[sizeof(u32)==4 ? 1 : -1];
typedef char check_ptr[sizeof(u8 *)==3 ? 1 : -1];
u16 Work(u16 x) {
 u16 a=x,b=x+1,c,i;
 for(i=0;i<8;++i) { c=a; a=b; b=c+1; }
 return a+b;
}
