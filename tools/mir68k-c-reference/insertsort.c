/* Equivalent of fixtures/runtime/tacle/insertsort/{insertsort.act,kernel.inc}.
 * Derived from TACLeBench / Sung-Soo Lim's SNU-RT Benchmark Suite.
 * May be used, modified and redistributed freely with that acknowledgement.
 * Match Action! widths and volatility; retain the original sorting algorithm.
 */
#include <stdint.h>

_Static_assert(sizeof(int16_t) == 2 && sizeof(uint32_t) == 4, "integer widths");
uint8_t command, result;
int32_t itersI, minI, maxI, itersA, minA, maxA;
uint32_t values[11], input[11];

void Initialize(uint32_t *source)
{
    volatile int16_t i;
    for (i = 0; i <= 10; ++i) values[i] = source[i];
}

void Init(void)
{
    uint32_t original[11] = {0, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2};
    itersI = 0; minI = 100000; maxI = 0;
    itersA = 0; minA = 100000; maxA = 0;
    Initialize(original);
}

uint8_t CheckResult(void)
{
    uint32_t sum = 0;
    for (int16_t i = 0; i <= 10; ++i) sum += values[i];
    return sum != 65;
}

void Sort(void)
{
    int16_t i = 2, j;
    uint32_t temp;
    itersI = 0;
    while (i <= 10) {
        ++itersI;
        j = i; itersA = 0;
        while (values[j] < values[j - 1]) {
            ++itersA;
            temp = values[j];
            values[j] = values[j - 1];
            values[j - 1] = temp;
            --j;
        }
        if (itersA < minA) minA = itersA;
        if (itersA > maxA) maxA = itersA;
        ++i;
    }
    if (itersI < minI) minI = itersI;
    if (itersI > maxI) maxI = itersI;
}

void Main(void)
{
    if (command == 0) Init();
    else Initialize(input);
    Sort();
    result = CheckResult();
}
