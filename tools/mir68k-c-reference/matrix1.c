/* Equivalent of fixtures/runtime/tacle/matrix1/{matrix1.act,kernel.inc}.
 * Derived from TACLeBench / Juan Martinez Velarde's DSP-Stone matrix1.
 * May be used, modified and redistributed freely.
 * Compile with -fwrapv for Action!'s signed 32-bit multiply/add wrapping.
 */
#include <stdint.h>

_Static_assert(sizeof(uint16_t) == 2 && sizeof(int32_t) == 4, "integer widths");
enum { Rows = 10, Inner = 10, Columns = 10 };
uint8_t command;
int16_t result;
int32_t checksum;
int32_t matrixA[Rows * Inner], matrixB[Inner * Columns], matrixC[Rows * Columns];

void PinDown(int32_t *a, int32_t *b, int32_t *c)
{
    volatile int32_t one = 1;
    uint16_t i;
    for (i = 0; i < Rows * Inner; ++i) a[i] = one;
    for (i = 0; i < Inner * Columns; ++i) b[i] = one;
    for (i = 0; i < Rows * Columns; ++i) c[i] = 0;
}

void Init(void) { PinDown(matrixA, matrixB, matrixC); }

int16_t CheckResult(void)
{
    checksum = 0;
    for (uint16_t i = 0; i < Rows * Columns; ++i) checksum += matrixC[i];
    return checksum == 1000 ? 0 : -1;
}

void Multiply(void)
{
    int32_t *pa = matrixA, *pb = matrixB, *pc = matrixC;
    for (uint16_t k = 0; k < Columns; ++k) {
        pa = matrixA;
        for (uint16_t i = 0; i < Rows; ++i) {
            pb = &matrixB[k * Inner];
            *pc = 0;
            for (uint16_t f = 0; f < Inner; ++f) {
                *pc += *pa * *pb;
                ++pa; ++pb;
            }
            ++pc;
        }
    }
}

void Main(void)
{
    if (command == 0) Init();
    Multiply();
    result = CheckResult();
}
