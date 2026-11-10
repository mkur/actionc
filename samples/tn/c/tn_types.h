#ifndef TN_TYPES_H
#define TN_TYPES_H
#include <stddef.h>
typedef unsigned char u8;
typedef unsigned int u16;
typedef signed int i16;
typedef u8 *Bytes;
typedef u16 *Words;
typedef struct PanelState {
    u8 files, nestLevel, drive, tagMask, selected, row;
    u16 directory;
    u8 taggedCount;
} PanelState;
typedef struct DirectoryRow {
    u8 tag, marker, name[8], separator, extension[3], separator2, sectors[3];
} DirectoryRow;
#define TN_ASSERT(name, condition) typedef char name[(condition) ? 1 : -1]
TN_ASSERT(byte_width, sizeof(u8)==1);
TN_ASSERT(word_width, sizeof(u16)==2);
TN_ASSERT(int_width, sizeof(i16)==2);
TN_ASSERT(pointer_width, sizeof(void*)==2);
TN_ASSERT(panel_size, sizeof(PanelState)==9);
TN_ASSERT(panel_files, offsetof(PanelState,files)==0);
TN_ASSERT(panel_level, offsetof(PanelState,nestLevel)==1);
TN_ASSERT(panel_drive, offsetof(PanelState,drive)==2);
TN_ASSERT(panel_mask, offsetof(PanelState,tagMask)==3);
TN_ASSERT(panel_selected, offsetof(PanelState,selected)==4);
TN_ASSERT(panel_row, offsetof(PanelState,row)==5);
TN_ASSERT(row_tag, offsetof(DirectoryRow,tag)==0);
TN_ASSERT(row_marker, offsetof(DirectoryRow,marker)==1);
TN_ASSERT(row_separator, offsetof(DirectoryRow,separator)==10);
TN_ASSERT(row_separator2, offsetof(DirectoryRow,separator2)==14);
TN_ASSERT(panel_directory, offsetof(PanelState,directory)==6);
TN_ASSERT(panel_tags, offsetof(PanelState,taggedCount)==8);
TN_ASSERT(row_size, sizeof(DirectoryRow)==18);
TN_ASSERT(row_name, offsetof(DirectoryRow,name)==2);
TN_ASSERT(row_extension, offsetof(DirectoryRow,extension)==11);
TN_ASSERT(row_sectors, offsetof(DirectoryRow,sectors)==15);
#define MEM(a) (*(u8 *)(u16)(a))
#define WORD(a) (*(u16 *)(u16)(a))
#define HW(a) (*(volatile u8 *)(u16)(a))
#endif
