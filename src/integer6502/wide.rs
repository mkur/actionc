//! Private 32-bit kernels. All branches are local; only Error is relocatable.
//! Inputs: $82..$85 and $C0..$C3. Result: $C4..$C7, little-endian.
//! Scratch: $82..$87, $C0..$C7, A/X/Y/P. The stack is balanced on return.
use super::*;

fn negate(base: u8) -> String {
    let mut source = String::from("SEC\n");
    for byte in base..base + 4 {
        source.push_str(&format!("LDA #0\nSBC ${byte:02X}\nSTA ${byte:02X}\n"));
    }
    source
}

fn copy(from: u8, to: u8) -> String {
    (0..4)
        .map(|i| format!("LDA ${:02X}\nSTA ${:02X}\n", from + i, to + i))
        .collect()
}

fn shift(base: u8, left: bool) -> String {
    if left {
        format!(
            "ASL ${base:02X}\nROL ${:02X}\nROL ${:02X}\nROL ${:02X}\n",
            base + 1,
            base + 2,
            base + 3
        )
    } else {
        format!(
            "LSR ${:02X}\nROR ${:02X}\nROR ${:02X}\nROR ${base:02X}\n",
            base + 3,
            base + 2,
            base + 1
        )
    }
}

fn pure_body(source: &str) -> Vec<u8> {
    let body = assemble(source, 0, InlineAsmMode::Analyzed).expect("valid wide integer kernel");
    assert!(
        body.relocations
            .iter()
            .all(|r| matches!(r.target, InlineAsmRelocationTarget::Absolute(_))),
        "pure kernel has no symbolic relocations"
    );
    body.bytes
}

pub(crate) fn multiply() -> Vec<u8> {
    let mut source = String::from(
        "CLD\nLDA #0\nSTA $C4\nSTA $C5\nSTA $C6\nSTA $C7\n\
         LDA $C0\nORA $C1\nORA $C2\nORA $C3\nBNE nonzero\nRTS\n\
         nonzero: LDA $C3\nBPL width\n",
    );
    // (-a) * (-b) == a * b modulo 2^32, including unsigned inputs and MIN.
    // Normalize a negative multiplier so sign-extended narrow values need
    // only their magnitude's rounds. All inputs already reside in scratch.
    source.push_str(&negate(0x82));
    source.push_str(&negate(0xC0));
    source.push_str(
        "width: LDA $C3\nBNE wide32\nLDA $C2\nBEQ narrow_check\n\
         LDX #24\nBNE wide_loop\nwide32: LDX #32\n",
    );
    source.push_str(&multiply_loop("wide", 4));
    source.push_str(
        "RTS\nnarrow_check: LDA $84\nORA $85\nBEQ narrow\n\
         LDA $84\nAND $85\nCMP #$FF\nBNE general_narrow\n\
         narrow: LDA $C0\nSTA $C4\nLDA $C1\nBEQ narrow_byte\nSTA $C5\n",
    );
    // For a zero-extended word, the 16x16 product is the complete result.
    // For an upper word of $FFFF, a = low16(a) - 65536 modulo 2^32:
    // subtract b from the product's upper word after multiplying low16(a).
    // This also admits -65536, and works after the paired negation above.
    source.push_str(&multiply_word_loop("narrow_word", 2));
    source.push_str("CLC\nBCC narrow_sign\nnarrow_byte:\n");
    source.push_str(&multiply_word_loop("narrow_byte", 1));
    source.push_str(
        "narrow_sign: LDA $85\nBPL narrow_done\n\
         SEC\nLDA $C6\nSBC $C0\nSTA $C6\nLDA $C7\nSBC $C1\nSTA $C7\n\
         narrow_done: RTS\ngeneral_narrow: LDA $C1\nBEQ byte\nLDX #16\n",
    );
    // Arbitrary 32-bit left operands still need every multiplicand/result lane.
    source.push_str(&multiply_loop("word", 2));
    source.push_str("RTS\nbyte: LDX #8\n");
    source.push_str(&multiply_loop("byte", 1));
    pure_body(&source)
}

fn multiply_word_loop(label: &str, multiplier_bytes: u8) -> String {
    // Rotate the partial product right instead of shifting a four-byte
    // multiplicand left. Its high byte stays in A; each set multiplier bit
    // adds just two bytes. C4.. holds the multiplier initially and the result
    // finally. C0/C1 remain intact for the signed high-word correction.
    // Seed carry with the first multiplier bit. Each rotate consumes the
    // addition's seventeenth bit and leaves the next multiplier bit in carry;
    // DEX/BNE preserve it, including when the addition was skipped.
    let partial_low = 0xC4 + multiplier_bytes;
    let mut source = format!("LDX #{}\nLDA #0\n", multiplier_bytes * 8);
    source.push_str(&format!("LSR ${:02X}\n", partial_low - 1));
    for byte in (0xC4..partial_low - 1).rev() {
        source.push_str(&format!("ROR ${byte:02X}\n"));
    }
    source.push_str(&format!(
        "{label}_loop: BCC {label}_rotate\nTAY\nCLC\n\
         LDA ${partial_low:02X}\nADC $82\nSTA ${partial_low:02X}\nTYA\nADC $83\n\
         {label}_rotate: ROR A\n",
    ));
    for byte in (0xC4..=partial_low).rev() {
        source.push_str(&format!("ROR ${byte:02X}\n"));
    }
    source.push_str(&format!(
        "DEX\nBNE {label}_loop\nSTA ${:02X}\n",
        partial_low + 1
    ));
    source
}

fn multiply_loop(label: &str, multiplier_bytes: u8) -> String {
    // The width dispatch proves all omitted multiplier bytes are zero. Begin
    // with LSR so the top retained byte receives a zero, then rotate down.
    // Multiplicand and result remain four bytes, including for narrow paths.
    let top = 0xC0 + multiplier_bytes - 1;
    let mut source = format!("{label}_loop: LSR ${top:02X}\n");
    for byte in (0xC0..top).rev() {
        source.push_str(&format!("ROR ${byte:02X}\n"));
    }
    source.push_str(&format!("BCC {label}_next\nCLC\n"));
    for i in 0..4 {
        source.push_str(&format!(
            "LDA ${:02X}\nADC ${:02X}\nSTA ${:02X}\n",
            0xC4 + i,
            0x82 + i,
            0xC4 + i
        ));
    }
    source.push_str(&format!("{label}_next:\n"));
    source.push_str(&shift(0x82, true));
    source.push_str(&format!("DEX\nBNE {label}_loop\n"));
    source
}

pub(crate) fn shift_body(left: bool) -> Vec<u8> {
    let mut source = String::from(
        "CLD\nLDA $C1\nORA $C2\nORA $C3\nBNE zero\nLDA $C0\nCMP #32\nBCS zero\nTAX\nBEQ result\nloop:\n",
    );
    source.push_str(&shift(0x82, left));
    source.push_str("DEX\nBNE loop\nresult:\n");
    source.push_str(&copy(0x82, 0xC4));
    source.push_str("CLC\nBCC done\nzero: LDA #0\nSTA $C4\nSTA $C5\nSTA $C6\nSTA $C7\ndone:\n");
    pure_body(&source)
}

pub(crate) fn division(signed: bool, remainder: bool) -> IntegerHelperBody {
    let mut source = String::from("CLD\nLDA $C0\nORA $C1\nORA $C2\nORA $C3\nBNE nonzero\n");
    source.push_str(&fault_source(RuntimeFault::DivisionByZero));
    source.push_str("nonzero:\n");
    if signed {
        source.push_str("LDA $85\nSTA $86\nEOR $C3\nSTA $87\nLDA $85\nBPL positive_left\n");
        source.push_str(&negate(0x82));
        source.push_str("positive_left: LDA $C3\nBPL positive_right\n");
        source.push_str(&negate(0xC0));
        source.push_str("positive_right:\n");
    }
    source.push_str("LDA #0\nSTA $C4\nSTA $C5\nSTA $C6\nSTA $C7\nLDX #32\ndivide:\n");
    source.push_str(&shift(0x82, true));
    // Preserve the 33rd remainder bit: overflow forces subtraction.
    source.push_str("ROL $C4\nROL $C5\nROL $C6\nROL $C7\nBCS subtract\n");
    for i in (0..4).rev() {
        source.push_str(&format!(
            "LDA ${:02X}\nCMP ${:02X}\nBCC next\n",
            0xC4 + i,
            0xC0 + i
        ));
        if i != 0 {
            source.push_str("BNE subtract\n");
        }
    }
    source.push_str("subtract: SEC\n");
    for i in 0..4 {
        source.push_str(&format!(
            "LDA ${:02X}\nSBC ${:02X}\nSTA ${:02X}\n",
            0xC4 + i,
            0xC0 + i,
            0xC4 + i
        ));
    }
    source.push_str("INC $82\nnext: DEX\nBNE divide\n");
    if !remainder {
        source.push_str(&copy(0x82, 0xC4));
    }
    if signed {
        source.push_str(if remainder { "LDA $86\n" } else { "LDA $87\n" });
        source.push_str("BPL result\n");
        source.push_str(&negate(0xC4));
        source.push_str("result:\n");
    }
    assemble_body(&source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_kernels_have_only_the_declared_error_relocation() {
        assert!(!multiply().is_empty());
        for left in [false, true] {
            assert!(!shift_body(left).is_empty());
        }
        for signed in [false, true] {
            for remainder in [false, true] {
                let body = division(signed, remainder);
                assert_eq!(
                    &body.bytes[body.error_operand - 6..body.error_operand],
                    &[0xA0, 101, 0x98, 0xA2, 0, 0x20]
                );
                assert_eq!(
                    &body.bytes[body.error_operand + 2..body.error_operand + 9],
                    &[0xD8, 0xA0, 101, 0x98, 0x38, 0xB0, 0xFE]
                );
            }
        }
    }
}
