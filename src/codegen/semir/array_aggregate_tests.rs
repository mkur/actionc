use super::array_execution_tests::{execute, outputs};

fn backing(memory: &[u8; 65536], length: usize) -> &[u8] {
    let address = u16::from_le_bytes([memory[0x0600], memory[0x0601]]) as usize;
    &memory[address..address + length]
}

#[test]
fn embedded_array_initializers_execute_recursive_leaf_order_and_partial_zero_fill() {
    let declaration = "TYPE Point=[BYTE tag CARD word] \
        TYPE Batch=[BYTE lead Point ARRAY points(2) INT ARRAY values(3) BYTE tail]";
    for local in [false, true] {
        for (variable, initializer, expected) in [
            (
                "Batch data",
                "[1 2 $2345 3 $6789 -1 -2 300 7]",
                vec![
                    1, 2, 0x45, 0x23, 3, 0x89, 0x67, 255, 255, 254, 255, 44, 1, 7,
                ],
            ),
            (
                "Batch data",
                "[1 2 $2345 3]",
                vec![1, 2, 0x45, 0x23, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            ),
            ("Batch ARRAY data", "[1 2 $2345 3 $6789 -1 -2 300 7 8 9]", {
                let mut bytes = vec![
                    1, 2, 0x45, 0x23, 3, 0x89, 0x67, 255, 255, 254, 255, 44, 1, 7, 8, 9,
                ];
                bytes.resize(28, 0);
                bytes
            }),
        ] {
            let target = if variable.contains("ARRAY") {
                "data(0)"
            } else {
                "data"
            };
            let object = format!("{variable}={initializer}");
            let source = if local {
                format!(
                    "{declaration} CARD address=$0600 PROC Main() {object} address=CARD(@{target}) RETURN"
                )
            } else {
                format!(
                    "{declaration} {object} CARD address=$0600 PROC Main() address=CARD(@{target}) RETURN"
                )
            };
            for (label, output) in outputs(&source) {
                let memory = execute(&output, |_| {});
                assert_eq!(
                    backing(&memory, expected.len()),
                    expected,
                    "{label}/local={local}/{source}"
                );
            }
        }
    }
}

#[test]
fn embedded_array_initializers_preserve_real_and_relocatable_scalar_leaves() {
    let source = "BYTE target TYPE Mixed=[BYTE lead REAL ARRAY values(2) CARD ARRAY addresses(2) BYTE tail] \
        Mixed data=[1 1.5 -2.25 @target @target+1 7] \
        CARD address=$0600,targetAddress=$0602 \
        PROC Main() address=CARD(@data) targetAddress=CARD(@target) RETURN";
    for (label, output) in outputs(source) {
        let memory = execute(&output, |_| {});
        let target = u16::from_le_bytes([memory[0x0602], memory[0x0603]]);
        let mut expected = vec![1];
        for literal in ["1.5", "-2.25"] {
            expected.extend(
                crate::atari_real::AtariReal::from_decimal(literal)
                    .unwrap()
                    .to_bytes(),
            );
        }
        expected.extend(target.to_le_bytes());
        expected.extend((target + 1).to_le_bytes());
        expected.push(7);
        assert_eq!(backing(&memory, expected.len()), expected, "{label}");
    }
}

#[test]
fn embedded_array_initializers_cover_page_boundary_lengths() {
    for (element, width) in [("BYTE", 1), ("CHAR", 1), ("INT", 2), ("CARD", 2)] {
        for length in [1, 2, 100, 127, 128, 129, 255, 256, 257] {
            let values = (0..length)
                .map(|index| ((index * 3 + 1) % 127).to_string())
                .collect::<Vec<_>>()
                .join(" ");
            let source = format!(
                "TYPE Buffer=[BYTE lead {element} ARRAY values({length}) BYTE tail] \
                Buffer data=[$A5 {values} $5A] CARD address=$0600 \
                PROC Main() address=CARD(@data) RETURN"
            );
            let mut expected = vec![0xA5];
            for index in 0..length {
                expected.push(((index * 3 + 1) % 127) as u8);
                if width == 2 {
                    expected.push(0);
                }
            }
            expected.push(0x5A);
            for (label, output) in outputs(&source) {
                let memory = execute(&output, |_| {});
                assert_eq!(
                    backing(&memory, expected.len()),
                    expected,
                    "{label}/{element}/{length}"
                );
            }
        }
    }
}
