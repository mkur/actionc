/* Source: TN.ACT MakeJmp and Handle; command bodies are separate translations. */
#include "tn.h"
#include "tn_text.h"
void MakeJmp(u8 command) {
    switch(command) {
    case CMD_VIEW:View();break;
    case CMD_ATTRIB:Attrib();break;
    case CMD_DELETE:Delete();break;
    case CMD_RENAME:Rename();break;
    case CMD_TAG_ALL:TagAll();break;
    case CMD_COPY:Copy();break;
    case CMD_MKDIR:MkDir();break;
    case CMD_FORMAT:Format();break;
#ifdef TN_DEBUG
    case CMD_DBG:Dbg();break;
#endif
    case CMD_QUIT:Quit();break;
    }
}
static void load_selected(void) {
    SwapScr();WORD(0x0A)=0x1B48;_StrNam(1,fname);WORD(0x86)=WORD(0x354);
    __asm volatile {
        ldx #0x10
        lda #0x1B
        pha
        lda #0x47
        pha
        lda #39
        sta 0x0352
        lda #0
        sta 0x0358
        sta 0x0359
        lda #4
        sta 0x035A
        jmp 0xE456
    }
}
void Handle(void) {
    u8 ch,command,d;
    do {
        Inv(active.row);validdir=TRUE;
        while(validdir) {
            Close(1);d=active.selected;
            ch=Range(text_bytes_a16a24cd,(u8)(active.files-1),&active.selected);
#ifdef TN_DEBUG
            command=Instr(text_vadrscmfgq,ch);
#else
            command=Instr(text_vadrscmfq,ch);
#endif
            if(active.files) {
                if(command!=CMD_NONE) {MakeJmp(command);if(ch=='C') break;}
                else if(ch==32) {
                    Tag(active.selected);
                    if(active.selected<(u8)(active.files-1)) {++active.selected;if(active.row<LAST_VISIBLE_FILE) ++active.row;}
                    UpdDis();
                } else if(ch==EOL) {
                    Convert((Bytes)v[active.selected]);
                    if(IsDirectory(active.selected)) {
                        if(active.nestLevel<MAX_PATH_DEPTH) {
                            dirsectors[active.nestLevel]=*currentDir;++active.nestLevel;g_j=Instr(fname,'.');
                            s=(Bytes)dirnames[active.nestLevel];s[0]=(u8)(g_j-3);
                            for(g_i=3;g_i<=g_j;++g_i) s[g_i-2]=fname[g_i];
                            Xio(1,CIO_CHDIR,0,0,fname);validdir=FALSE;
                        }
                    } else load_selected();
                } else if(d<active.selected) {if(active.row<LAST_VISIBLE_FILE) ++active.row;UpdDis();}
                else if(d>active.selected) {if(active.row!=0) --active.row;UpdDis();}
                else if(ch=='>') GoTo((u8)(active.files-1));
                else if(ch=='<') GoTo(0);
            } else if(command>CMD_COPY) MakeJmp(command);
            if(((ch=='+' || ch==0x1E) && winnum==1) || ((ch=='*' || ch==0x1F) && winnum==0)) ch=0x7F;
            if(ch==0x0F) {SwapScr();Getchar();SwapScr();}
            else if(ch==0x7F) {Inv(active.row);SwapWin();Path();break;}
            else if(ch=='N') {NewDrive();if(ans!=ESC) break;}
            else if(ch>='1' && ch<='9') {SetWin(winnum,(u8)(ch-'0'));break;}
            else if(ch==0x12) validdir=FALSE;
            else if(ch==ESC) {
                if(active.nestLevel) {--active.nestLevel;*currentDir=dirsectors[active.nestLevel];validdir=FALSE;}
            } else if(ch==125 || ch==255) {
                SwapScr();if(ch==255) ch=EOL;PutD(0,ch);Getchar();SwapScr();
            }
            CloseAll();
        }
        if(validdir==FALSE) Dir();
    } while(1);
}
