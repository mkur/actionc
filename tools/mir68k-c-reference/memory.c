/* Minimal freestanding support for GCC-generated aggregate copies/clears.
 * Its emitted code and execution are included in the comparison.
 */
#include <stddef.h>
void *memcpy(void *destination, const void *source, size_t size)
{
    unsigned char *out = destination;
    const unsigned char *in = source;
    while (size--) *out++ = *in++;
    return destination;
}
void *memset(void *destination, int value, size_t size)
{
    unsigned char *out = destination;
    while (size--) *out++ = (unsigned char)value;
    return destination;
}
