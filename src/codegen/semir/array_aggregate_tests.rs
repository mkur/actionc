use super::array_execution_tests::{execute, outputs};

#[test]
fn embedded_subobject_pointer_initializers_relocate_static_bases() {
    for local in [false, true] {
        let declarations = "TYPE Part=[BYTE tag INT ARRAY values(129) BYTE tail] \
            TYPE Wrapper=[BYTE lead Part ARRAY items(2) BYTE tail] \
            Part data Wrapper whole Part ARRAY rows(2) Part fixed=$70F0 \
            INT POINTER p=data.values,q=@data.values,r=INT POINTER(@data.values),s=@data.values(1) \
            INT POINTER nested=@whole.items(1).values(128),row=@rows(1).values(128),absolute=@fixed.values(128)";
        let probes = "CARD a=$0600,b=$0602,c=$0604,d=$0606,e=$0608,f=$060A,g=$060C \
            CARD dataAddress=$0610,wholeAddress=$0612,rowsAddress=$0614";
        let body = "a=CARD(p) b=CARD(q) c=CARD(r) d=CARD(s) \
            e=CARD(nested) f=CARD(row) g=CARD(absolute) \
            dataAddress=CARD(@data) wholeAddress=CARD(@whole) rowsAddress=CARD(@rows(0)) RETURN";
        let source = if local {
            format!("{probes} PROC Main() {declarations} {body}")
        } else {
            format!("{declarations} {probes} PROC Main() {body}")
        };
        for (backend, output) in outputs(&source) {
            let ram = execute(&output, |_| {});
            let word = |address: usize| u16::from_le_bytes([ram[address], ram[address + 1]]);
            for (address, expected) in [
                (0x600, word(0x610) + 1),
                (0x602, word(0x610) + 1),
                (0x604, word(0x610) + 1),
                (0x606, word(0x610) + 3),
                (0x608, word(0x612) + 518),
                (0x60A, word(0x614) + 517),
                (0x60C, 0x71F1),
            ] {
                assert_eq!(
                    word(address),
                    expected,
                    "{backend}, local={local}, probe={address:x}"
                );
            }
        }
    }
}

#[test]
fn embedded_subobject_list_initializers_share_symbol_addend_relocations() {
    let source = "TYPE Part=[BYTE tag INT ARRAY values(129) BYTE tail] \
        Part data Part ARRAY rows(2) \
        CARD ARRAY refs=[@data.values @data.values(1) @rows(1).values(128)+1] \
        BYTE ARRAY halves=[<@data.values(1) >@data.values(1)] \
        TYPE Table=[CARD ARRAY addresses(3) BYTE tail] \
        Table tableData=[@data.values @data.values(1) @rows(1).values(128)+1 $A5] \
        CARD a=$0600,b=$0602,c=$0604,d=$0606,e=$0608 \
        PROC Main() a=CARD(@data) b=CARD(@rows(0)) c=CARD(@refs(0)) \
          d=CARD(@halves(0)) e=CARD(@tableData) RETURN";
    for (backend, output) in outputs(source) {
        let ram = execute(&output, |_| {});
        let word = |address: usize| u16::from_le_bytes([ram[address], ram[address + 1]]);
        let data = word(0x600);
        let rows = word(0x602);
        let expected = [data + 1, data + 3, rows + 518]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        for base in [word(0x604), word(0x608)] {
            assert_eq!(
                &ram[usize::from(base)..usize::from(base) + 6],
                &expected,
                "{backend}"
            );
        }
        assert_eq!(word(usize::from(word(0x606))), data + 3, "{backend}");
        assert_eq!(ram[usize::from(word(0x608)) + 6], 0xA5, "{backend}");
    }
}

#[test]
fn static_address_lists_resolve_local_absolute_and_alias_backings() {
    let source = "BYTE ARRAY globalData(4)=[11 22 33 44] CARD result=$0600 \
        PROC Main() BYTE absolute=$7000 BYTE local=[55] \
          BYTE localAlias=local BYTE globalAlias=globalData+1 \
          CARD ARRAY refs=[@absolute @localAlias @globalAlias] result=CARD(@refs(0)) RETURN";
    for (backend, output) in outputs(source) {
        let ram = execute(&output, |ram| ram[0x7000] = 0xA5);
        let word = |address: usize| u16::from_le_bytes([ram[address], ram[address + 1]]);
        let base = usize::from(word(0x600));
        let addresses = [word(base), word(base + 2), word(base + 4)];
        assert_eq!(addresses[0], 0x7000, "{backend}");
        assert_eq!(
            addresses.map(|address| ram[usize::from(address)]),
            [0xA5, 55, 22],
            "{backend}"
        );
    }
}

#[test]
fn embedded_subobject_addresses_support_inferred_initialized_record_arrays() {
    let source = "TYPE Part=[BYTE tag INT ARRAY values(2)] Part ARRAY rows=[1 2 3 4 5 6] \
        INT POINTER p=@rows(1).values(1) CARD result=$0600,base=$0602 \
        PROC Main() result=CARD(p) base=CARD(@rows(0)) RETURN";
    for (backend, output) in outputs(source) {
        let ram = execute(&output, |_| {});
        let word = |address: usize| u16::from_le_bytes([ram[address], ram[address + 1]]);
        assert_eq!(word(0x600), word(0x602) + 8, "{backend}");
        assert_eq!(word(usize::from(word(0x600))), 6, "{backend}");
    }
}

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
