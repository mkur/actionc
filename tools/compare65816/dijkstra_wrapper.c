/* Host protocol for the pinned TACLeBench source; no algorithm changes. */
typedef char check_int[(sizeof(int)==2)?1:-1];
typedef char check_long[(sizeof(long)==4)?1:-1];
typedef char check_ptr[(sizeof(void *)==3)?1:-1];
unsigned char command,done;
int startNode,endNode,result;
struct _QITEM *headAddress;
int argNode,argDist,argPrev,outNode,outDist,outPrev,seedNext;
unsigned long layoutNodeSize,layoutNodeDist,layoutNodePrev;
unsigned long layoutQueueSize,layoutQueueNode,layoutQueueDist,layoutQueuePrev,layoutQueueNext;
unsigned long layoutRowSize,layoutRowCost,layoutIntSize,layoutPointerSize;
#define OFF(T,F) ((unsigned long)&((T *)0)->F)
void QueryLayout(void) {
 layoutNodeSize=sizeof(struct _NODE);layoutNodeDist=OFF(struct _NODE,dist);layoutNodePrev=OFF(struct _NODE,prev);
 layoutQueueSize=sizeof(struct _QITEM);layoutQueueNode=OFF(struct _QITEM,node);layoutQueueDist=OFF(struct _QITEM,dist);layoutQueuePrev=OFF(struct _QITEM,prev);layoutQueueNext=OFF(struct _QITEM,next);
 layoutRowSize=sizeof(matrix[0]);layoutRowCost=0;layoutIntSize=sizeof(int);layoutPointerSize=sizeof(headAddress);
}
void Main(void) {
 if(command==255) {QueryLayout();return;}
 head=headAddress;
 if(command==0) {Init();Benchmark();result=ChecksumResult();}
 else if(command==1) {Init();queueNext=seedNext;result=Find(startNode,endNode);}
 else if(command==2) {result=Enqueue(argNode,argDist,argPrev);}
 else {Dequeue(&outNode,&outDist,&outPrev);result=0;}
 headAddress=head;done=0xa5;
}
