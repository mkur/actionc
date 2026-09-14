; Huffman decoder by David Bourgin (1994), from TACLeBench huff_dec.
; Action! adaptation, 2026. See README.md and huff_dec.c for attribution/terms.
; The host supplies valid streams and zeroed code/tree storage. No recursion.

BYTE FUNC EndOfData()
RETURN(inputPos>=inputLength)

BYTE FUNC ReadByte()
  BYTE value
  value=encoded(inputPos)
  inputPos==+1
RETURN(value)

PROC WriteByte(BYTE value)
  decoded(outputPos)=value
  outputPos==+1
RETURN

BYTE FUNC ReadBit()
  IF bitsLeft#0 THEN
    bitsLeft==-1
    RETURN(BYTE((reservoir RSH bitsLeft)&1))
  ELSE
    reservoir=ReadByte()
    bitsLeft=7
    RETURN(BYTE((reservoir RSH 7)&1))
  FI
RETURN(0)

LONGCARD FUNC ReadBits(CARD n)
  LONGCARD value
  CARD i
  value=0 i=n
  WHILE i#0 DO
    WHILE (bitsLeft<9) AND (EndOfData()=0) DO
      reservoir=(reservoir LSH 8)+ReadByte()
      bitsLeft==+8
    OD
    IF i>=bitsLeft THEN
      value=(value LSH bitsLeft)+reservoir
      i==-bitsLeft
      bitsLeft=0
    ELSE
      value=(value LSH i)+((reservoir RSH (bitsLeft-i))&((LONGCARD(1) LSH i)-1))
      bitsLeft==-i
      i=0
    FI
  OD
RETURN(value)

PROC ReadHeader(Code ARRAY table)
  CARD i,j,byteIndex
  FOR i=0 TO 256 DO
    table(i).length=0
    FOR j=0 TO 31 DO table(i).bits(j)=0 OD
  OD
  IF ReadBit()#0 THEN
    FOR i=0 TO 255 DO table(i).present=ReadBit() OD
  ELSE
    i=ReadBits(5)+1
    WHILE i#0 DO
      table(ReadBits(8)).present=1
      i==-1
    OD
  FI
  table(256).present=1
  FOR i=0 TO 256 DO
    IF table(i).present#0 THEN
      IF ReadBit()#0 THEN j=ReadBits(8)+1
      ELSE j=ReadBits(5)+1 FI
      table(i).length=j
      byteIndex=(j-1) RSH 3
      IF (j&7)#0 THEN
        table(i).bits(byteIndex)=ReadBits(j&7)
        j==-(j&7)
        byteIndex==-1
      FI
      WHILE j>=8 DO
        table(i).bits(byteIndex)=ReadBits(8)
        j==-8 byteIndex==-1
      OD
    FI
  OD
RETURN

Tree POINTER FUNC BuildTree(Code ARRAY table Tree ARRAY pool)
  CARD i,j,top
  Tree POINTER root,current
  top=0
  root=@pool(top) top==+1
  root.symbol=257 root.left=0 root.right=0
  FOR i=0 TO 256 DO
    current=root j=table(i).length
    WHILE j#0 DO
      IF (table(i).bits((j-1) RSH 3)&(1 LSH ((j-1)&7)))#0 THEN
        IF current.left=0 THEN
          current.left=@pool(top) top==+1
          current=current.left
          current.left=0 current.right=0
        ELSE current=current.left FI
      ELSE
        IF current.right=0 THEN
          current.right=@pool(top) top==+1
          current=current.right
          current.left=0 current.right=0
        ELSE current=current.right FI
      FI
      IF j=1 THEN current.symbol=i
      ELSE current.symbol=257 FI
      j==-1
    OD
  OD
  nodeCount=top
RETURN(root)

PROC Decode()
  Tree POINTER root,current
  IF EndOfData()=0 THEN
    ReadHeader(codes)
    root=BuildTree(codes,heap)
    rootAddress=root
    DO
      current=root
      WHILE current.symbol=257 DO
        IF ReadBit()#0 THEN current=current.left
        ELSE current=current.right FI
      OD
      IF current.symbol<=255 THEN WriteByte(current.symbol) FI
    UNTIL current.symbol=256 OD
  FI
RETURN
