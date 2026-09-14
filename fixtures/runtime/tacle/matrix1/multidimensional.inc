; TACLeBench matrix1 by Juan Martinez Velarde, originally from DSP-Stone.
; Action! adaptation, 2026. See README.md and matrix1.c for provenance.
; Modern fixed multidimensional variant. The last coordinate varies fastest.
; B and C use (column,row) coordinates to retain the reference byte order.

PROC PinDown(LONGINT ARRAY a,b,c)
  CARD i
  VOLATILE LONGINT one=[1]
  FOR i=0 TO Rows*Inner-1 DO a(i)=one OD
  FOR i=0 TO Inner*Columns-1 DO b(i)=one OD
  FOR i=0 TO Rows*Columns-1 DO c(i)=0 OD
RETURN

PROC Init()
  PinDown(matrixA,matrixB,matrixC)
RETURN

INT FUNC CheckResult()
  CARD row,column
  checksum=0
  FOR column=0 TO Columns-1 DO
    FOR row=0 TO Rows-1 DO checksum==+matrixC(column,row) OD
  OD
  IF checksum=1000 THEN RETURN(0) FI
RETURN(-1)

PROC Multiply()
  CARD row,column,k
  FOR column=0 TO Columns-1 DO
    FOR row=0 TO Rows-1 DO
      matrixC(column,row)=0
      FOR k=0 TO Inner-1 DO
        matrixC(column,row)==+matrixA(row,k)*matrixB(column,k)
      OD
    OD
  OD
RETURN
