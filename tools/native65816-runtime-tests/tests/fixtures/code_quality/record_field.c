/* Exact unsigned widths; compile with -mhuge -ptr24. */
typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned long u32;
typedef char check_u16[sizeof(u16)==2 ? 1 : -1];
typedef char check_u32[sizeof(u32)==4 ? 1 : -1];
typedef char check_ptr[sizeof(u8 *)==3 ? 1 : -1];
struct Pair { u16 low,high; };
typedef char check_pair[sizeof(struct Pair)==4 ? 1 : -1];
u16 Work(const struct Pair *p) { return p->high; }
