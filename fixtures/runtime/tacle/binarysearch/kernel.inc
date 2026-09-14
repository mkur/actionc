; TACLeBench binary search by Sung-Soo Lim, from the SNU-RT Benchmark Suite.
; Action! adaptation, 2026. See README.md and binarysearch.c for provenance.
; The tests derive BYTE, INT, and CARD record/search variants from this source.

PROC InitSeed()
  seed=0
RETURN

LONGINT FUNC RandomInteger()
  seed=(seed*133+81) MOD 8095
RETURN(seed)

PROC Init()
  INT i
  InitSeed()
  FOR i=0 TO 14 DO
    data(i).key=RandomInteger()
    data(i).value=RandomInteger()
  OD
RETURN

LONGINT FUNC Search(LONGINT x)
  LONGINT fvalue
  INT mid,up,low
  low=0 up=14 fvalue=-1
  WHILE low<=up DO
    mid=(low+up) RSH 1
    IF data(mid).key=x THEN
      up=low-1
      fvalue=data(mid).value
    ELSEIF data(mid).key>x THEN
      up=mid-1
    ELSE
      low=mid+1
    FI
  OD
RETURN(fvalue)
