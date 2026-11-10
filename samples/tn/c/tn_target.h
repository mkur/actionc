#ifndef TN_TARGET_H
#define TN_TARGET_H
/* TN LIB.ACT owns $E0/$E1, $E6-$E9 and $FE/$FF. Keep compiler
   globals at $EA-$FD and shorten the volatile temporary bank to 16 bytes.
   Oscar's saved register bank starts at $B5; setjmp uses 32 saved bytes. */
#pragma register(__sregs, 0xB5)
#pragma register(__addr, 0xD5)
#pragma register(__sp, 0xD9)
#pragma register(__fp, 0xDB)
#pragma region(zeropage, 0xEA, 0xFE, , , {zeropage})
#pragma section(startup, 0)
#pragma region(startup, 0x2C00, 0x2C80, , , {startup})
#ifndef TN_RAM_END
#define TN_RAM_END 0x9C00
#endif
#pragma region(main, 0x2C80, TN_RAM_END, , , {code, data, bss, heap, stack})
#pragma heapsize(0)
#pragma stacksize(512)
#endif
