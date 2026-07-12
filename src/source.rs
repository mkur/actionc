#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

pub fn decode_source(bytes: &[u8]) -> String {
    let mut source = String::with_capacity(bytes.len());

    for &byte in bytes {
        match byte {
            // Atari text files commonly use EOL $9B instead of LF.
            0x9b => source.push('\n'),
            _ => source.push(byte as char),
        }
    }

    source
}

pub fn source_char_byte(ch: char) -> Option<u8> {
    let value = ch as u32;
    (value <= 0xFF).then_some(value as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_atascii_eol() {
        assert_eq!(
            decode_source(b"PROC main()\x9bRETURN()"),
            "PROC main()\nRETURN()"
        );
    }

    #[test]
    fn preserves_atascii_high_bit_bytes() {
        let source = decode_source(&[b'"', 0xD4, 0xEF, b'"']);

        assert_eq!(source.chars().collect::<Vec<_>>(), vec!['"', 'Ô', 'ï', '"']);
        assert_eq!(source_char_byte('Ô'), Some(0xD4));
        assert_eq!(source_char_byte('€'), None);
    }
}
