import unittest
from build import check_ranges, image_guard_ranges


def guard(base, amount, fault, short):
    word = amount.to_bytes(2, 'little')
    long = lambda n: n.to_bytes(3, 'little')
    if short:
        return (bytes.fromhex('3b aa c5 46 90 06 f0 04 5c')+long(base+22)
                +bytes.fromhex('38 e9')+word+bytes.fromhex('90 04 c5 44 b0 07 a9')
                +word+b'\x5c'+long(fault))
    return (bytes.fromhex('3b aa c5 46 b0 04 5c')+long(base+20)
            +bytes.fromhex('d0 04 5c')+long(base+20)+b'\x5c'+long(base+38)
            +bytes.fromhex('38 e9')+word+bytes.fromhex('b0 04 5c')+long(base+38)
            +bytes.fromhex('c5 44 90 04 5c')+long(base+45)+b'\xa9'+word+b'\x5c'+long(fault))


class GuardRanges(unittest.TestCase):
    def test_complete_moved_forms_and_mixed_ranges(self):
        for base, fault in [(0x10000, 0x48000), (0x61ff00, 0x718123)]:
            for amount in [0, 4, 19, 255, 65535]:
                a = guard(base, amount, fault, False)
                b = guard(base+len(a), amount, fault, True)
                self.assertEqual(check_ranges(a+b, base, fault), [[base, base+45], [base+45, base+74]])
                self.assertEqual(sum(hi-lo for lo, hi in check_ranges(a+b, base, fault)), 74)

    def test_every_mutated_byte_and_truncation_is_rejected(self):
        for short in [False, True]:
            code = guard(0x10000, 19, 0x48000, short)
            for at in range(len(code)):
                changed = bytearray(code); changed[at] ^= 1
                self.assertEqual(check_ranges(changed, 0x10000), [], (short, at))
            for end in range(len(code)):
                self.assertEqual(check_ranges(code[:end], 0x10000), [])
            self.assertEqual(check_ranges(code, 0x10001), [])
            self.assertEqual(check_ranges(code, 0x10000, 0x48001), [])
            # Overwriting one guard with another admits only the complete one.
            overlap = code[:4]+guard(0x10004, 7, 0x48000, short)
            self.assertEqual(check_ranges(overlap, 0x10000), [[0x10004, 0x10004+len(code)]])

    def test_bank_wrap_and_missing_guard_inventory_fail(self):
        self.assertEqual(check_ranges(guard(0x1fff0, 0, 0x48000, True), 0x1fff0), [])
        image = dict(stack_overflow=0x48000, routines=[dict(address=0x10000, calls=[{}])])
        segment = dict(address=0x10000, bytes=list(guard(0x10000, 0, 0x48000, True)))
        with self.assertRaises(AssertionError): image_guard_ranges(image, segment)
        image['routines'][0]['calls'] = []
        self.assertEqual(image_guard_ranges(image, segment), [[0x10000, 0x1001d]])
        segment['bytes'][4] ^= 1
        with self.assertRaises(AssertionError): image_guard_ranges(image, segment)


if __name__ == '__main__':
    unittest.main()
