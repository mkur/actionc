/* Exec816 execlists.act translated directly; packed 24-bit pointers. */
#ifdef __CALYPSI__
#define PTR __huge
#define LINK __far24
#include <stddef.h>
#define OFFSET(T,F) offsetof(T,F)
#else
#define PTR
#define LINK
#define OFFSET(T,F) __offsetof(T,F)
#endif
typedef unsigned char u8;
typedef unsigned long u32;
typedef struct MinNode { struct MinNode LINK *mln_Succ, *mln_Pred; } MinNode;
typedef struct MinList { MinNode LINK *mlh_Head, *mlh_Tail, *mlh_TailPred; } MinList;
typedef struct Node { struct Node LINK *ln_Succ, *ln_Pred; u8 ln_Type, ln_Pri; u8 LINK *ln_Name; } Node;
typedef struct List { Node LINK *lh_Head, *lh_Tail, *lh_TailPred; u8 lh_Type, lh_Pad; } List;
#ifdef __CALYPSI__
/* Calypsi's front end validates far24 sizeof as four; its target lowering
   emits three. The builder checks these emitted target layout constants. */
const unsigned execlists_layout[] = {
 sizeof(void LINK *), sizeof(MinNode), sizeof(MinList), sizeof(Node), sizeof(List),
 OFFSET(Node,ln_Pred), OFFSET(Node,ln_Type), OFFSET(Node,ln_Pri), OFFSET(Node,ln_Name),
 OFFSET(List,lh_Tail), OFFSET(List,lh_TailPred), OFFSET(List,lh_Type), OFFSET(List,lh_Pad)
};
#else
typedef char check_ptr[sizeof(void LINK *) == 3 ? 1 : -1];
typedef char check_min_node[sizeof(MinNode) == 6 ? 1 : -1];
typedef char check_min_list[sizeof(MinList) == 9 ? 1 : -1];
typedef char check_node[sizeof(Node) == 11 ? 1 : -1];
typedef char check_list[sizeof(List) == 11 ? 1 : -1];
typedef char check_node_offsets[OFFSET(Node,ln_Pred)==3 && OFFSET(Node,ln_Type)==6 && OFFSET(Node,ln_Pri)==7 && OFFSET(Node,ln_Name)==8 ? 1 : -1];
typedef char check_list_offsets[OFFSET(List,lh_Tail)==3 && OFFSET(List,lh_TailPred)==6 && OFFSET(List,lh_Type)==9 && OFFSET(List,lh_Pad)==10 ? 1 : -1];
#endif
void NewList(List PTR *chain) {
 chain->lh_Head=(Node LINK *)((Node PTR *)&chain->lh_Tail);
 chain->lh_Tail=(Node LINK *)((Node PTR *)0);
 chain->lh_TailPred=(Node LINK *)((Node PTR *)&chain->lh_Head);
}
void NewMinList(MinList PTR *chain) { NewList((List PTR *)chain); }
u8 IsListEmpty(List PTR *chain) { return chain->lh_Head==(Node PTR *)&chain->lh_Tail; }
u8 IsMinListEmpty(MinList PTR *chain) { return IsListEmpty((List PTR *)chain); }
void AddHead(List PTR *chain, Node PTR *item) {
 Node PTR *first=chain->lh_Head;
 item->ln_Pred=(Node LINK *)((Node PTR *)&chain->lh_Head);
 item->ln_Succ=(Node LINK *)(first);
 first->ln_Pred=(Node LINK *)(item);
 chain->lh_Head=(Node LINK *)(item);
}
void AddTail(List PTR *chain, Node PTR *item) {
 Node PTR *last=chain->lh_TailPred;
 item->ln_Succ=(Node LINK *)((Node PTR *)&chain->lh_Tail);
 item->ln_Pred=(Node LINK *)(last);
 last->ln_Succ=(Node LINK *)(item);
 chain->lh_TailPred=(Node LINK *)(item);
}
void Insert(List PTR *chain, Node PTR *item, Node PTR *predecessor) {
 Node PTR *following;
 if (predecessor==(Node PTR *)0 || predecessor==(Node PTR *)&chain->lh_Head) { AddHead(chain,item); return; }
 if (predecessor==(Node PTR *)&chain->lh_Tail) { AddTail(chain,item); return; }
 following=predecessor->ln_Succ;
 item->ln_Pred=(Node LINK *)(predecessor);
 item->ln_Succ=(Node LINK *)(following);
 following->ln_Pred=(Node LINK *)(item);
 predecessor->ln_Succ=(Node LINK *)(item);
}
void Remove(Node PTR *item) {
 Node PTR *previous=item->ln_Pred, *following=item->ln_Succ;
 previous->ln_Succ=(Node LINK *)(following);
 following->ln_Pred=(Node LINK *)(previous);
}
Node PTR *RemHead(List PTR *chain) {
 Node PTR *item=chain->lh_Head;
 if (item->ln_Succ==(Node PTR *)0) return (Node PTR *)0;
 Remove(item);
 return item;
}
Node PTR *RemTail(List PTR *chain) {
 Node PTR *item=chain->lh_TailPred;
 if (item->ln_Pred==(Node PTR *)0) return (Node PTR *)0;
 Remove(item);
 return item;
}
void Enqueue(List PTR *chain, Node PTR *item) {
 Node PTR *cursor=chain->lh_Head;
 while (cursor->ln_Succ!=(Node PTR *)0) {
  if ((item->ln_Pri ^ 0x80) > (cursor->ln_Pri ^ 0x80)) { Insert(chain,item,cursor->ln_Pred); return; }
  cursor=cursor->ln_Succ;
 }
 AddTail(chain,item);
}
Node PTR *FindName(List PTR *chain, u8 PTR *name) {
 Node PTR *item=chain->lh_Head;
 u8 PTR *left,*right;
 while (item->ln_Succ!=(Node PTR *)0) {
  left=item->ln_Name; right=name;
  if (left!=(u8 PTR *)0) {
   while (*left==*right) {
    if (*left==0) return item;
    ++left;
    ++right;
   }
  }
  item=item->ln_Succ;
 }
 return (Node PTR *)0;
}
