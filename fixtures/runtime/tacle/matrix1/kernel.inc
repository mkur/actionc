; TACLeBench matrix1 by Juan Martinez Velarde, originally from DSP-Stone.
; Action! adaptation, 2026. See README.md and matrix1.c for provenance.
; A is row-major; B and C are column-major. All arrays are flat.

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
  CARD i
  checksum=0
  FOR i=0 TO Rows*Columns-1 DO checksum==+matrixC(i) OD
  IF checksum=1000 THEN RETURN(0) FI
RETURN(-1)

PROC Multiply()
  LONGINT POINTER pa,pb,pc
  CARD f,i,k
  pa=matrixA pb=matrixB pc=matrixC
  FOR k=0 TO Columns-1 DO
    pa=matrixA
    FOR i=0 TO Rows-1 DO
      pb=@matrixB(k*Inner)
      pc^=0
      FOR f=0 TO Inner-1 DO
        pc^==+pa^*pb^
        pa==+ElementBytes pb==+ElementBytes
      OD
      pc==+ElementBytes
    OD
  OD
RETURN
