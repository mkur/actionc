/* Kernel from c-bench-64; only benchmark/UI wrappers removed. */
#define __data
unsigned char CRC8(const unsigned char *data, unsigned int length)
{
    // CRC-8/GSM-A
    unsigned char __data crc;
    unsigned char __data tmp;
    unsigned int __data i;
    unsigned char __data j;

    crc = 0;
    for (i = 0; i < length; i++)
    {
        tmp = *data;
        crc ^= tmp;

        for (j = 8; j; j--)
        {
            if ((crc & 0x80) != 0)
                crc = ((crc << 1) ^ 0x1d);
            else
                crc <<= 1;
        }
        data++;
    }
    return crc;
}
