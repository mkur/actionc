; Huffman decoder by David Bourgin (1994), from TACLeBench huff_dec.
; Action! adaptation, 2026. See README.md and huff_dec.c for attribution/terms.
; The host supplies valid streams and zeroed code/tree storage. No recursion.

TYPE Code=[BYTE ARRAY bits(32) CARD length BYTE present]
TYPE Tree=[CARD symbol Tree POINTER left,right]
