/* Equivalent of matrix1/{multidimensional.act,multidimensional.inc}.
 * Derived from TACLeBench / Juan Martinez Velarde's DSP-Stone matrix1.
 * May be used, modified and redistributed freely. -fwrapv defines the signed
 * 32-bit multiply/add wrapping; no signed shifts are used.
 */
#include <stdint.h>

enum { Rows = 10, Inner = 10, Columns = 10 };
uint8_t command;
int16_t result;
int32_t checksum;
int32_t matrixABacking[Rows][Inner], matrixBBacking[Columns][Inner];
int32_t matrixCBacking[Columns][Rows];
int32_t (*matrixA)[Inner] = matrixABacking;
int32_t (*matrixB)[Inner] = matrixBBacking;
int32_t (*matrixC)[Rows] = matrixCBacking;

void PinDown(int32_t a[Rows][Inner], int32_t b[Columns][Inner],
             int32_t c[Columns][Rows])
{
    volatile int32_t one = 1;
    uint16_t i;
    /* Retain the flat initializer's iteration and volatile-read order while
     * indexing real C rows; flat pointer arithmetic past row zero is not used. */
    for (i = 0; i < Rows * Inner; ++i) a[i / Inner][i % Inner] = one;
    for (i = 0; i < Inner * Columns; ++i) b[i / Inner][i % Inner] = one;
    for (i = 0; i < Rows * Columns; ++i) c[i / Rows][i % Rows] = 0;
}

void Init(void) { PinDown(matrixA, matrixB, matrixC); }

int16_t CheckResult(void)
{
    uint16_t row, column;
    checksum = 0;
    for (column = 0; column < Columns; ++column)
        for (row = 0; row < Rows; ++row) checksum += matrixC[column][row];
    return checksum == 1000 ? 0 : -1;
}

void Multiply(void)
{
    uint16_t row, column, k;
    for (column = 0; column < Columns; ++column) {
        for (row = 0; row < Rows; ++row) {
            matrixC[column][row] = 0;
            for (k = 0; k < Inner; ++k)
                matrixC[column][row] += matrixA[row][k] * matrixB[column][k];
        }
    }
}

void Main(void)
{
    if (command == 0) Init();
    Multiply();
    result = CheckResult();
}
