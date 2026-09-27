/* Kernel from c-bench-64; only benchmark/UI wrappers removed. */
#define __data
unsigned int CRC16(const unsigned char *data, unsigned int length)
{
    /* CRC-16/XMODEM */
    unsigned int crc;
    unsigned char extract;
    unsigned int i;
    unsigned char j;

    crc = 0;
    for (i = 0; i < length; i++)
    {
        extract = *data;
#ifdef KICKC
        > crc = > crc ^ extract;
#else
        crc ^= (extract << 8);
#endif

        for (j = 8; j; j--)
        {
            if ((crc & 0x8000) != 0)
                crc = (crc << 1) ^ 0x1021;
            else
                crc <<= 1;
        }
        data++;
    }
    return crc;
}
