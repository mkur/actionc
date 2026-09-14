; TACLeBench/MiBench Dijkstra port. Upstream author unknown; GNU GPL.
; Revision c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08. See README.md.
; The host supplies the graph and reads complete node/queue state from RAM.

CONST NumNodes=100,QueueSize=1000,NoNode=9999
TYPE Node=[INT dist,prev]
TYPE QueueItem=[INT node INT dist INT prev QueueItem POINTER next]
TYPE Row=[BYTE ARRAY cost(NumNodes)]
