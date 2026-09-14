; SHA-0 port of TACLeBench kernel/sha, by Peter C. Gutmann and Uwe Hollerbach.
; Upstream revision c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08. GNU LGPL.
; See README.md for attribution, reference adaptations, and host protocol.
; LONGCARD keeps every hash operation unsigned and exactly 32 bits wide.

PROC Transform()
  BYTE i
  LONGCARD a,b,c,d,e,temp

  FOR i=0 TO 15 DO
    w(i)=data(i)
  OD
  ; SHA-0's schedule has no extra one-bit rotation (unlike SHA-1).
  FOR i=16 TO 79 DO
    w(i)=w(i-3) XOR w(i-8) XOR w(i-14) XOR w(i-16)
  OD
  a=digest(0) b=digest(1) c=digest(2) d=digest(3) e=digest(4)

  FOR i=0 TO 19 DO
    temp=((a LSH 5) OR (a RSH 27))+
      ((b AND c) OR ((b XOR $FFFFFFFF) AND d))+e+w(i)+$5A827999
    e=d d=c c=(b LSH 30) OR (b RSH 2) b=a a=temp
  OD
  FOR i=20 TO 39 DO
    temp=((a LSH 5) OR (a RSH 27))+
      (b XOR c XOR d)+e+w(i)+$6ED9EBA1
    e=d d=c c=(b LSH 30) OR (b RSH 2) b=a a=temp
  OD
  FOR i=40 TO 59 DO
    temp=((a LSH 5) OR (a RSH 27))+
      ((b AND c) OR (b AND d) OR (c AND d))+e+w(i)+$8F1BBCDC
    e=d d=c c=(b LSH 30) OR (b RSH 2) b=a a=temp
  OD
  FOR i=60 TO 79 DO
    temp=((a LSH 5) OR (a RSH 27))+
      (b XOR c XOR d)+e+w(i)+$CA62C1D6
    e=d d=c c=(b LSH 30) OR (b RSH 2) b=a a=temp
  OD
  digest(0)==+a digest(1)==+b digest(2)==+c
  digest(3)==+d digest(4)==+e
RETURN

; Decode four input bytes explicitly, independently of target byte order.
; Reading each complete word before storing also permits aliased array views.
PROC DecodeBlock()
  BYTE i,index
  FOR i=0 TO 15 DO
    index=i LSH 2
    data(i)=(LONGCARD(blockBytes(index)) LSH 24) OR
      (LONGCARD(blockBytes(index+1)) LSH 16) OR
      (LONGCARD(blockBytes(index+2)) LSH 8) OR
      LONGCARD(blockBytes(index+3))
  OD
RETURN

PROC Init()
  BYTE i
  digest(0)=$67452301 digest(1)=$EFCDAB89 digest(2)=$98BADCFE
  digest(3)=$10325476 digest(4)=$C3D2E1F0
  countLo=0 countHi=0
  FOR i=0 TO 15 DO data(i)=0 OD
RETURN

; As upstream, preceding updates must contain complete blocks; only the last
; update may have a partial block. Main supplies the entire message at once.
PROC Update(BYTE POINTER buffer CARD count)
  BYTE i
  IF countLo+(LONGCARD(count) LSH 3)<countLo THEN
    countHi==+1
  FI
  countLo==+(LONGCARD(count) LSH 3)
  countHi==+(LONGCARD(count) RSH 29)
  WHILE count>=64 DO
    FOR i=0 TO 63 DO blockBytes(i)=buffer(i) OD
    DecodeBlock()
    Transform()
    buffer==+64 count==-64
  OD
  i=0
  WHILE i<count DO
    blockBytes(i)=buffer(i)
    i==+1
  OD
RETURN

PROC Final()
  BYTE count
  LONGCARD loBits,hiBits
  loBits=countLo hiBits=countHi
  count=BYTE((loBits RSH 3) AND $3F)
  blockBytes(count)=$80
  count==+1
  IF count>56 THEN
    WHILE count<64 DO blockBytes(count)=0 count==+1 OD
    DecodeBlock()
    Transform()
    count=0
  FI
  WHILE count<56 DO blockBytes(count)=0 count==+1 OD
  DecodeBlock()
  data(14)=hiBits data(15)=loBits
  Transform()
RETURN
