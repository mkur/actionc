/* Faithful high-level translations. Source: TN.ACT Strcpy..Sort, Convert;
   LIB.ACT Putchar/Print/PrintE. Numeric address conversion for Convert's
   byte displacement retains the Atari address role. Native C parameter homes
   have stronger isolation than Action!'s observable routine-static homes. */
#include "tn.h"
Bytes buffer,s;
Words v,dirsectors,currentDir;
u16 dirnames[PATH_LEVELS];
u8 fname[15];
u16 linebuf,dummy;
PanelState active;
u8 copyflag,validdir,ans,h,winnum;
void Strcpy(Bytes dst,Bytes src) { MovePage(dst,src,(u8)(src[0]+1)); }
void Strcat(Bytes dst,Bytes src) {
    MovePage(dst+(u16)dst[0]+1,src+1,src[0]);
    dst[0]=(u8)(dst[0]+src[0]);
}
u8 Instr(Bytes text,u8 ch) {
    for(g_i=1;g_i<=text[0];++g_i) {
        if(text[g_i]==ch) return g_i;
        if(g_i==255) break;
    }
    return 0;
}
void Putchar(u8 ch) { CalcAdr(); *screen=Internal(ch); ++kx; }
void Print(Bytes text) {
    u8 i;
    for(i=1;i<=text[0];++i) { Putchar(text[i]); if(i==255) break; }
}
void PrintE(Bytes text) { Print(text); kx=zx; ++ky; }
void PrintFname(Bytes text) {
    u8 i;
    for(i=3;i<=text[0];++i) { Putchar(text[i]); if(i==255) break; }
}
u8 Fnamecmp(Bytes left,Bytes right) {
    ++left; ++right;
    if(*left!=*right) { if(*left==':') return 0; else if(*right==':') return 1; }
    ++left; ++right;
    while(*left==*right) { ++left; ++right; }
    if(*left>*right) return 1; else return 0;
}
void Sort(void) {
    u8 gap;u16 h;
    gap=active.files>>1;
    while(gap) {
        for(g_i=gap;g_i<active.files;++g_i) {
            g_j=(u8)(g_i-gap);
            while(g_j<0x80) {
                if(Fnamecmp((Bytes)v[g_j],(Bytes)v[(u8)(g_j+gap)])) {
                    h=v[g_j];v[g_j]=v[(u8)(g_j+gap)];v[(u8)(g_j+gap)]=h;
                    g_j=(u8)(g_j-gap);
                } else break;
            }
        }
        gap>>=1;
    }
}
void Convert(Bytes text) {
    fname[1]='D';fname[2]=':';text=(Bytes)((u16)text-1);g_i=3;
    while(g_i<11 && text[g_i]!=0) { fname[g_i]=Ascii(text[g_i]); ++g_i; }
    fname[g_i]='.';g_j=(u8)(g_i+1);g_i=12;
    while(g_i<15 && text[g_i]!=0) { fname[g_j]=Ascii(text[g_i]);++g_i;++g_j; }
    fnamelen=(u8)(g_j-1);
}
