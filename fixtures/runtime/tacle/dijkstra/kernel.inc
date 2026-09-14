; TACLeBench/MiBench Dijkstra port. Upstream author unknown; GNU GPL.
; Revision c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08. See README.md.
; The host supplies the graph and reads complete node/queue state from RAM.

PROC Init()
  INT i,k
  VOLATILE INT x=[0]
  FOR i=0 TO NumNodes-1 DO
    FOR k=0 TO NumNodes-1 DO
      matrix(i).cost(k)==XOR x
    OD
  OD
  queueCount=0 queueNext=0 head=0 checksum=0
RETURN

INT FUNC ChecksumResult()
  IF checksum=25 THEN RETURN(0) FI
RETURN(-1)

INT FUNC Enqueue(INT node,dist,prev)
  QueueItem POINTER item,last
  item=@items(queueNext)
  last=head
  ; Preserve upstream's increment-before-check: slot 999 is never written.
  queueNext==+1
  IF queueNext>=QueueSize THEN RETURN(-1) FI
  item.node=node item.dist=dist item.prev=prev item.next=0
  IF last=0 THEN
    head=item
  ELSE
    WHILE last.next#0 DO last=last.next OD
    last.next=item
  FI
  queueCount==+1
RETURN(0)

PROC Dequeue(INT POINTER node,dist,prev)
  IF head#0 THEN
    node^=head.node dist^=head.dist prev^=head.prev
    head=head.next
    queueCount==-1
  FI
RETURN

INT FUNC QueueLength()
RETURN(queueCount)

INT FUNC Find(INT first,last)
  INT ch,prev,node,cost,dist,i
  node=0 dist=0
  FOR ch=0 TO NumNodes-1 DO
    nodes(ch).dist=NoNode nodes(ch).prev=NoNode
  OD
  ; Upstream leaves every distance at NoNode when first=last.
  IF first#last THEN
    nodes(first).dist=0 nodes(first).prev=NoNode
    IF Enqueue(first,0,NoNode)=-1 THEN RETURN(-1) FI
    WHILE QueueLength()>0 DO
      Dequeue(@node,@dist,@prev)
      FOR i=0 TO NumNodes-1 DO
        cost=matrix(node).cost(i)
        ; Kept from C, although a BYTE weight cannot equal 9999.
        IF cost#NoNode THEN
          IF (nodes(i).dist=NoNode) OR (nodes(i).dist>cost+dist) THEN
            nodes(i).dist=dist+cost nodes(i).prev=node
            IF Enqueue(i,dist+cost,node)=-1 THEN RETURN(-1) FI
          FI
        FI
      OD
    OD
  FI
RETURN(0)

PROC Benchmark()
  INT i,j
  j=NumNodes/2
  FOR i=0 TO 19 DO
    j=j MOD NumNodes
    IF Find(i,j)=-1 THEN
      checksum==-1
      RETURN
    ELSE
      checksum==+nodes(j).dist
    FI
    queueNext=0
    j==+1
  OD
RETURN
