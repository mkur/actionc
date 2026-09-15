/* Integer JPEG DCT, derived from TACLeBench / the Independent JPEG Group.
 * Copyright (C) 1991-1994, Thomas G. Lane; modified by Steven Li.
 * C comparison adaptation, 2026: fixed-width unsigned bit patterns, defined
 * wrapping/sign extension, flat and mutable row-pointer layouts, optional
 * intermediate-state capture. This software is based in part on the work of
 * the Independent JPEG Group. The unmodified copyright, permission and
 * no-warranty terms are in fixtures/runtime/tacle/jfdctint/README (LEGAL ISSUES).
 * Keep that README with any redistribution of this derived source.
 */
#include <stdint.h>
_Static_assert(sizeof(uint32_t) == 4 && sizeof(uint16_t) == 2, "integer widths");
enum { ConstBits = 13, Pass1Bits = 2 };
#if DCT_SHAPED
uint32_t blockBacking[8][8];
uint32_t (*block)[8] = blockBacking;
#define FLAT(i) block[(i) / 8][(i) % 8]
#define ROW(k) block[line][k]
#define COLUMN(k) block[k][line]
#else
uint32_t block[64];
#define FLAT(i) block[i]
#define ROW(k) p[k]
#define COLUMN(k) p[8 * (k)]
#endif
uint32_t checksum;
int16_t result;

/* Unsigned operations define every 32-bit input, including negative signed
 * bit patterns and overflow. -fwrapv alone would not define signed shifts. */
uint32_t Descale(uint32_t value, uint8_t bits)
{
    value += UINT32_C(1) << (bits - 1);
    if (value & UINT32_C(0x80000000)) return ~((~value) >> bits);
    return value >> bits;
}

void Init(void)
{
    uint32_t seed = 1;
#if DCT_SHAPED
    uint8_t row, column;
    for (row = 0; row < 8; ++row) {
        for (column = 0; column < 8; ++column) {
            seed = (seed * 133 + 81) % 65535;
            block[row][column] = seed;
        }
    }
#else
    uint8_t i;
    for (i = 0; i < 64; ++i) {
        seed = (seed * 133 + 81) % 65535;
        block[i] = seed;
    }
#endif
}

#ifdef ACTIONC_REFERENCE_CAPTURE
uint8_t testCommand, testShift;
uint32_t testInput[64], testInitial[64], testRows[64];
void CaptureRows(void)
{
    for (uint8_t i = 0; i < 64; ++i) testRows[i] = FLAT(i);
}
#endif

void Dct(void)
{
    uint32_t tmp0,tmp1,tmp2,tmp3,tmp4,tmp5,tmp6,tmp7;
    uint32_t tmp10,tmp11,tmp12,tmp13,z1,z2,z3,z4,z5;
    uint8_t line;
#if !DCT_SHAPED
    uint32_t *p = block;
#endif
    /* Eight adjacent values per row; same operation order and coefficients. */
    for (line = 0; line < 8; ++line) {
        tmp0=ROW(0)+ROW(7);
        tmp7=ROW(0) - ROW(7);
        tmp1=ROW(1)+ROW(6);
        tmp6=ROW(1) - ROW(6);
        tmp2=ROW(2)+ROW(5);
        tmp5=ROW(2) - ROW(5);
        tmp3=ROW(3)+ROW(4);
        tmp4=ROW(3) - ROW(4);
        tmp10=tmp0+tmp3;
        tmp13=tmp0 - tmp3;
        tmp11=tmp1+tmp2;
        tmp12=tmp1 - tmp2;
        ROW(0)=((tmp10+tmp11) << Pass1Bits);
        ROW(4)=((tmp10 - tmp11) << Pass1Bits);
        z1=(tmp12+tmp13)*4433;
        ROW(2)=Descale(z1+tmp13*6270, ConstBits-Pass1Bits);
        ROW(6)=Descale(z1+tmp12*(-UINT32_C(15137)), ConstBits-Pass1Bits);
        z1=tmp4+tmp7;
        z2=tmp5+tmp6;
        z3=tmp4+tmp6;
        z4=tmp5+tmp7;
        z5=(z3+z4)*9633;
        tmp4=tmp4*2446;
        tmp5=tmp5*16819;
        tmp6=tmp6*25172;
        tmp7=tmp7*12299;
        z1=z1*(-UINT32_C(7373));
        z2=z2*(-UINT32_C(20995));
        z3=z3*(-UINT32_C(16069));
        z4=z4*(-UINT32_C(3196));
        z3+=z5;
        z4+=z5;
        ROW(7)=Descale(tmp4+z1+z3, ConstBits-Pass1Bits);
        ROW(5)=Descale(tmp5+z2+z4, ConstBits-Pass1Bits);
        ROW(3)=Descale(tmp6+z2+z3, ConstBits-Pass1Bits);
        ROW(1)=Descale(tmp7+z1+z4, ConstBits-Pass1Bits);
#if !DCT_SHAPED
        p += 8;
#endif
    }
#ifdef ACTIONC_REFERENCE_CAPTURE
    CaptureRows();
#endif
#if !DCT_SHAPED
    p = block;
#endif
    /* Eight values per column, with eight-element spacing in the flat view. */
    for (line = 0; line < 8; ++line) {
        tmp0=COLUMN(0)+COLUMN(7);
        tmp7=COLUMN(0) - COLUMN(7);
        tmp1=COLUMN(1)+COLUMN(6);
        tmp6=COLUMN(1) - COLUMN(6);
        tmp2=COLUMN(2)+COLUMN(5);
        tmp5=COLUMN(2) - COLUMN(5);
        tmp3=COLUMN(3)+COLUMN(4);
        tmp4=COLUMN(3) - COLUMN(4);
        tmp10=tmp0+tmp3;
        tmp13=tmp0 - tmp3;
        tmp11=tmp1+tmp2;
        tmp12=tmp1 - tmp2;
        COLUMN(0)=Descale(tmp10+tmp11, Pass1Bits);
        COLUMN(4)=Descale(tmp10 - tmp11, Pass1Bits);
        z1=(tmp12+tmp13)*4433;
        COLUMN(2)=Descale(z1+tmp13*6270, ConstBits+Pass1Bits);
        COLUMN(6)=Descale(z1+tmp12*(-UINT32_C(15137)), ConstBits+Pass1Bits);
        z1=tmp4+tmp7;
        z2=tmp5+tmp6;
        z3=tmp4+tmp6;
        z4=tmp5+tmp7;
        z5=(z3+z4)*9633;
        tmp4=tmp4*2446;
        tmp5=tmp5*16819;
        tmp6=tmp6*25172;
        tmp7=tmp7*12299;
        z1=z1*(-UINT32_C(7373));
        z2=z2*(-UINT32_C(20995));
        z3=z3*(-UINT32_C(16069));
        z4=z4*(-UINT32_C(3196));
        z3+=z5;
        z4+=z5;
        COLUMN(7)=Descale(tmp4+z1+z3, ConstBits+Pass1Bits);
        COLUMN(5)=Descale(tmp5+z2+z4, ConstBits+Pass1Bits);
        COLUMN(3)=Descale(tmp6+z2+z3, ConstBits+Pass1Bits);
        COLUMN(1)=Descale(tmp7+z1+z4, ConstBits+Pass1Bits);
#if !DCT_SHAPED
        p += 1;
#endif
    }
}

int16_t CheckResult(void)
{
    checksum = 0;
#if DCT_SHAPED
    uint8_t row, column;
    for (row = 0; row < 8; ++row)
        for (column = 0; column < 8; ++column) checksum += block[row][column];
#else
    for (uint8_t i = 0; i < 64; ++i) checksum += block[i];
#endif
    return checksum == UINT32_C(1668124) ? 0 : -1;
}

void Main(void)
{
#ifdef ACTIONC_REFERENCE_CAPTURE
    uint8_t i;
    if (testCommand == 0) Init();
    else for (i = 0; i < 64; ++i) FLAT(i) = testInput[i];
    for (i = 0; i < 64; ++i) testInitial[i] = FLAT(i);
    if (testCommand == 2)
        for (i = 0; i < 64; ++i) FLAT(i) = Descale(FLAT(i), testShift);
    else Dct();
#else
    Init();
    Dct();
#endif
    result = CheckResult();
}
