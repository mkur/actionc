/* Kernel from c-bench-64; only benchmark/UI wrappers removed. */
#define __data
unsigned long CRC32(const unsigned char *data, unsigned int length)
{
    /* CRC-32/CKSUM */
    unsigned char extract;
    unsigned int i;
    unsigned char j;
    unsigned long crc;
    //  const unsigned char *end=data+length;

    crc = 0;

    for (i = 0; i < length; i++)
    {
        //  while(data!=end) {
        extract = *data;
        crc ^= (((unsigned long)extract) << 24);

        for (j = 8; j; j--)
        {
            if ((crc & 0x80000000) != 0)
            {
                //       crc=(crc<<1)^0x04c11db7;
                crc <<= 1;
                crc ^= 0x04c11db7;
            }
            else
                crc <<= 1;
        }
        data++;
    }
    crc ^= 0xffffffff;
    return crc;
}
