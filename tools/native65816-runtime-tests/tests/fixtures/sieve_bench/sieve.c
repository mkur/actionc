/* Original c-bench-64 kernel; benchmark/UI wrappers removed. */
#define SIZE 8191
#define EXPECTED 1900

char flags[SIZE];


unsigned int sieve(unsigned int n)
{
    unsigned int i, prime, k, count;

    unsigned int size = n;

    count = 1;

    for (i = 0; i < size; i++)
        flags[i] = 1;

    for (i = 0; i < size; i++)
    {
        if (flags[i])
        {
#ifdef KICKC
            prime = i * 2 + 3;
#else
            prime = i + i + 3;
#endif
            k = i + prime;
            while (k < size)
            {
                flags[k] = 0;
                k += prime;
            }
            count++;
        }
    }

    return count;
}
