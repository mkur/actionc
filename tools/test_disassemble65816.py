"""Instruction-boundary checks for native word arithmetic and mode tracking."""
import unittest

from disassemble65816 import disassemble


def listing(code):
    return disassemble(dict(format='actionc-65816-image', version=3,
                            abi='action65816.native.v1', segments=[dict(
                                executable=True, address=0x18000, bytes=code)]))


class WordArithmetic(unittest.TestCase):
    def test_stack_operands_remain_one_encoded_byte_in_both_modes(self):
        result = listing(bytes.fromhex('63 fe e3 04 e2 20 63 02 e3 ff c2 20 a9 34 12 6b'))
        self.assertEqual(result.splitlines(), [
            '018000  63 FE       ADC $FE,S',
            '018002  E3 04       SBC $04,S',
            '018004  E2 20       SEP #$20',
            '018006  63 02       ADC $02,S',
            '018008  E3 FF       SBC $FF,S',
            '01800A  C2 20       REP #$20',
            '01800C  A9 34 12    LDA #$1234',
            '01800F  6B          RTL',
        ])

    def test_immediate_width_changes_do_not_consume_the_next_opcode(self):
        result = listing(bytes.fromhex('69 ff 00 e9 00 80 e2 20 69 ff e9 80 6b'))
        self.assertIn('018000  69 FF 00    ADC #$00FF', result)
        self.assertIn('018003  E9 00 80    SBC #$8000', result)
        self.assertIn('018008  69 FF       ADC #$FF', result)
        self.assertIn('01800A  E9 80       SBC #$80', result)
        self.assertIn('01800C  6B          RTL', result)

    def test_truncated_arithmetic_is_rejected(self):
        for code in ('63', 'e3', '69 ff', 'e9 00', 'e2 20 63'):
            with self.subTest(code=code), self.assertRaisesRegex(ValueError, 'truncated'):
                listing(bytes.fromhex(code))


class WordComparisons(unittest.TestCase):
    def test_stack_and_immediate_cmp_follow_accumulator_width(self):
        result = listing(bytes.fromhex('c3 fe c9 00 80 e2 20 c3 ff c9 ff c2 20 c9 ff 00 6b'))
        self.assertEqual(result.splitlines(), [
            '018000  C3 FE       CMP $FE,S',
            '018002  C9 00 80    CMP #$8000',
            '018005  E2 20       SEP #$20',
            '018007  C3 FF       CMP $FF,S',
            '018009  C9 FF       CMP #$FF',
            '01800B  C2 20       REP #$20',
            '01800D  C9 FF 00    CMP #$00FF',
            '018010  6B          RTL',
        ])

    def test_truncated_cmp_is_rejected(self):
        for code in ('c3', 'c9 ff', 'e2 20 c3', 'e2 20 c9', 'e2 20 c9 ff c2 20 c9 00'):
            with self.subTest(code=code), self.assertRaisesRegex(ValueError, 'truncated'):
                listing(bytes.fromhex(code))


if __name__ == '__main__':
    unittest.main()
