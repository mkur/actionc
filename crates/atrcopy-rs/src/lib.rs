pub fn atascii_to_ascii(bytes: &[u8]) -> String {
    let mut output = String::new();
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            0x9b => {
                output.push('\n');
                index += 1;
            }
            0x1b => {
                output.push_str("\\{ESC}");
                index += 1;
            }
            0x7d => {
                output.push_str("\\{CLEAR}");
                index += 1;
            }
            b'\\' => {
                output.push_str("\\{$5C}");
                index += 1;
            }
            0x20..=0x7e => {
                output.push(byte as char);
                index += 1;
            }
            0x80..=0xff if inverse_escape_char(byte).is_some() => {
                let start = index;
                index += 1;
                while index < bytes.len() && inverse_escape_char(bytes[index]).is_some() {
                    index += 1;
                }
                output.push_str("\\{INV:");
                for &inverse in &bytes[start..index] {
                    output.push(inverse_escape_char(inverse).expect("checked inverse char"));
                }
                output.push('}');
            }
            _ => {
                output.push_str(&format!("\\{{${byte:02X}}}"));
                index += 1;
            }
        }
    }
    output
}

pub fn ascii_to_atascii(text: &str) -> Result<Vec<u8>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut output = Vec::new();
    let mut index = 0usize;

    while index < chars.len() {
        let ch = chars[index];
        if ch == '\r' {
            if chars.get(index + 1) == Some(&'\n') {
                index += 1;
            }
            output.push(0x9b);
            index += 1;
            continue;
        }
        if ch == '\n' {
            output.push(0x9b);
            index += 1;
            continue;
        }
        if ch == '\\' && chars.get(index + 1) == Some(&'{') {
            let body_start = index + 2;
            let Some(body_end) = chars[body_start..]
                .iter()
                .position(|candidate| *candidate == '}')
            else {
                return Err("unterminated ATASCII escape".to_string());
            };
            let body_end = body_start + body_end;
            let body: String = chars[body_start..body_end].iter().collect();
            output.extend(decode_ascii_escape(&body)?);
            index = body_end + 1;
            continue;
        }
        if !ch.is_ascii() {
            return Err(format!(
                "non-ASCII character `{ch}` must be written as an ATASCII escape"
            ));
        }
        output.push(ch as u8);
        index += 1;
    }

    Ok(output)
}

fn inverse_escape_char(byte: u8) -> Option<char> {
    if byte < 0x80 {
        return None;
    }
    let low = byte & 0x7f;
    if matches!(low, 0x20..=0x7e) && low != b'}' {
        Some(low as char)
    } else {
        None
    }
}

fn decode_ascii_escape(body: &str) -> Result<Vec<u8>, String> {
    if let Some(hex) = body.strip_prefix('$') {
        return decode_hex_byte(hex).map(|byte| vec![byte]);
    }
    if let Some(hex) = body
        .strip_prefix("CHAR:$")
        .or_else(|| body.strip_prefix("char:$"))
    {
        return decode_hex_byte(hex).map(|byte| vec![byte]);
    }
    if let Some(text) = body
        .strip_prefix("INV:")
        .or_else(|| body.strip_prefix("inv:"))
    {
        if text.is_empty() {
            return Err("inverse ATASCII escape requires at least one character".to_string());
        }
        let mut bytes = Vec::new();
        for ch in text.chars() {
            if !ch.is_ascii() {
                return Err(format!("cannot inverse non-ASCII character `{ch}`"));
            }
            bytes.push((ch as u8) | 0x80);
        }
        return Ok(bytes);
    }

    named_ascii_escape(body)
        .map(|byte| vec![byte])
        .ok_or_else(|| format!("unknown ATASCII escape `{body}`"))
}

fn decode_hex_byte(hex: &str) -> Result<u8, String> {
    if hex.len() != 2 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!(
            "ATASCII byte escape requires two hex digits, got `{hex}`"
        ));
    }
    u8::from_str_radix(hex, 16).map_err(|_| format!("invalid ATASCII byte `${hex}`"))
}

fn named_ascii_escape(name: &str) -> Option<u8> {
    match name.to_ascii_uppercase().as_str() {
        "RETURN" | "EOL" | "CR" => Some(0x9b),
        "ESC" | "ESCAPE" => Some(0x1b),
        "CLEAR" | "CLS" => Some(0x7d),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{ascii_to_atascii, atascii_to_ascii};

    #[test]
    fn encodes_atascii_line_endings_and_named_controls() {
        assert_eq!(
            atascii_to_ascii(&[b'A', 0x9b, 0x1b, 0x7d]),
            "A\n\\{ESC}\\{CLEAR}"
        );
    }

    #[test]
    fn encodes_inverse_runs_and_exact_bytes() {
        assert_eq!(
            atascii_to_ascii(&[0xc1, 0xe2, 0xfd, 0x00]),
            "\\{INV:Ab}\\{$FD}\\{$00}"
        );
    }

    #[test]
    fn escapes_backslash_to_keep_roundtrip_unambiguous() {
        let encoded = atascii_to_ascii(br"\{RETURN}");
        assert_eq!(encoded, "\\{$5C}{RETURN\\{CLEAR}");
        assert_eq!(ascii_to_atascii(&encoded).unwrap(), br"\{RETURN}");
    }

    #[test]
    fn decodes_modern_ascii_encoding_to_atascii() {
        assert_eq!(
            ascii_to_atascii("A\n\\{RETURN}\\{INV:Ab}\\{$00}\\{CLEAR}").unwrap(),
            vec![b'A', 0x9b, 0x9b, 0xc1, 0xe2, 0x00, 0x7d]
        );
    }
}
