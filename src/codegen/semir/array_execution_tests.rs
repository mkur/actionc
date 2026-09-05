//! Experimental end-to-end backend coverage without enabling public profiles.
use crate::codegen::{CodegenOutput, CodegenProfile};
use crate::lexer::tokenize;
use crate::parser::parse;
use crate::runtime::Runtime;
use crate::semantic::{self, SemanticOptions};

#[path = "../test_cpu.rs"]
mod cpu;

fn outputs(source: &str) -> Vec<(String, CodegenOutput)> {
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(
        &ast,
        SemanticOptions {
            embedded_record_arrays: true,
            ..SemanticOptions::modern()
        },
    )
    .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
    let semir = semantic::ir::lower_program(&ast, &model);
    let nir = crate::nir::lower_program(&semir);
    let nir = crate::nir::optimize_program(&nir).unwrap();
    let mut outputs = Vec::new();
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let classic = match runtime {
            Runtime::ActionCart => crate::codegen::generate_semir_profile_at_origin(
                &semir,
                0x3000,
                CodegenProfile::Modern,
            ),
            Runtime::Standalone => crate::codegen::generate_semir_standalone_profile_at_origin(
                &semir,
                0x3000,
                CodegenProfile::Modern,
            ),
        }
        .unwrap_or_else(|errors| panic!("classic/{runtime:?}: {source}\n{errors:#?}"));
        outputs.push((format!("classic/{runtime:?}"), classic));
        let mir = crate::mir6502::generate_output_with_config_and_runtime(
            &nir,
            0x3000,
            &crate::mir6502::Mir6502Config::default(),
            runtime,
        )
        .unwrap_or_else(|errors| panic!("MIR6502/{runtime:?}: {source}\n{errors:#?}"));
        outputs.push((format!("MIR6502/{runtime:?}"), mir));
    }
    outputs
}

fn execute(output: &CodegenOutput, initialize: impl FnOnce(&mut [u8; 65536])) -> [u8; 65536] {
    let mut memory = [0u8; 65536];
    if output.map.runtime == Runtime::ActionCart {
        // These arithmetic-only fixtures use the initial OSS type-15 mapping;
        // no bank switching or OS services are involved.
        let cart = include_bytes!("../../../roms/action.rom");
        memory[0xA000..0xB000].copy_from_slice(&cart[0x1010..0x2010]);
        memory[0xB000..0xC000].copy_from_slice(&cart[0x10..0x1010]);
    }
    let origin = usize::from(output.origin);
    memory[origin..origin + output.bytes.len()].copy_from_slice(&output.bytes);
    initialize(&mut memory);
    cpu::run_memory(&mut memory, usize::from(output.run_address));
    memory
}

fn assert_bytes(actual: &[u8], expected: &[u8], label: &str) {
    assert_eq!(actual.len(), expected.len());
    let mismatches: Vec<_> = actual
        .iter()
        .zip(expected)
        .enumerate()
        .filter(|(_, (actual, expected))| actual != expected)
        .take(16)
        .collect();
    assert!(
        mismatches.is_empty(),
        "{label}: byte mismatches {mismatches:?}"
    );
}

#[test]
fn embedded_array_execution_direct_pointer_and_nested_elements() {
    let source = "TYPE Buffer=[BYTE lead INT ARRAY x(129),y(129) BYTE tail] \
        TYPE Envelope=[BYTE lead Buffer payload BYTE tail] \
        Buffer data=$5001 Buffer POINTER p Envelope box=$5801 \
        CARD index=$0600 INT result=$0602 \
        PROC Main() p=data data.x(index)=$1234 p.y(index)=data.x(index) \
        box.payload.y(index)=p.y(index) box.payload.y(index)==+1 \
        result=box.payload.y(index) RETURN";
    for (label, output) in outputs(source) {
        for index in [0u16, 1, 127, 128] {
            let memory = execute(&output, |memory| {
                memory[0x5000..0x5B00].fill(0xA5);
                memory[0x0600..0x0602].copy_from_slice(&index.to_le_bytes());
            });
            let mut expected = vec![0xA5; 0xB00];
            for (offset, value) in [
                (2 + index * 2, 0x1234u16),
                (260 + index * 2, 0x1234),
                (0x905 + index * 2, 0x1235),
            ] {
                expected[usize::from(offset)..usize::from(offset) + 2]
                    .copy_from_slice(&value.to_le_bytes());
            }
            assert_eq!(&memory[0x0602..0x0604], &[0x35, 0x12], "{label}/{index}");
            assert_bytes(
                &memory[0x5000..0x5B00],
                &expected,
                &format!("{label}/{index}"),
            );
        }
    }
}

#[test]
fn embedded_array_execution_decay_captures_effectful_indexes_once() {
    let source = "TYPE Buffer=[INT ARRAY x(100),y(100)] Buffer ARRAY rows(3) \
        INT POINTER p INT result=$0600 BYTE calls=$0602 \
        BYTE FUNC Row() calls==+1 RETURN(1) \
        PROC Take(INT ARRAY values) result=values(2) RETURN \
        PROC Main() calls=0 rows(1).y(2)=$4321 p=rows(Row()).y Take(p) \
        Take(rows(Row()).y) RETURN";
    for (label, output) in outputs(source) {
        let memory = execute(&output, |_| {});
        assert_eq!(&memory[0x0600..0x0603], &[0x21, 0x43, 2], "{label}");
    }
}

#[test]
fn embedded_array_execution_preserves_destination_across_index_and_rhs_calls() {
    let source = "TYPE Buffer=[INT ARRAY x(129),y(129)] \
        Buffer first=$5001,second=$5803 Buffer POINTER p BYTE calls=$0600 \
        INT result=$0602 BYTE FUNC Index() calls==+1 p=second RETURN(128) \
        INT FUNC Value() calls==+1 p=second RETURN(4660) \
        PROC Main() calls=0 p=first p.x(Index())=Value() \
        result=first.x(128) RETURN";
    for (label, output) in outputs(source) {
        let memory = execute(&output, |memory| memory[0x5000..0x5B00].fill(0xA5));
        let mut expected = vec![0xA5; 0xB00];
        expected[257..259].copy_from_slice(&0x1234u16.to_le_bytes());
        assert_bytes(&memory[0x5000..0x5B00], &expected, &label);
        assert_eq!(memory[0x0600], 2, "{label}");
        assert_eq!(&memory[0x0602..0x0604], &[0x34, 0x12], "{label}");
    }
}

#[test]
fn embedded_array_execution_scalar_boundaries() {
    for (ty, width, literal, value) in [
        ("BYTE", 1usize, "$A6", 0xA6u16),
        ("CHAR", 1, "65", 65),
        ("INT", 2, "-1234", (-1234i16) as u16),
        ("CARD", 2, "$BEEF", 0xBEEF),
    ] {
        for bound in [1usize, 2, 100, 127, 128, 129, 255, 256, 257] {
            let source = format!(
                "TYPE Buffer=[BYTE head {ty} ARRAY x({bound}),y({bound}) BYTE tail] \
                Buffer data=$6001 Buffer POINTER p CARD index=$0600 {ty} result=$0602 \
                PROC Main() p=data data.x(index)={literal} p.y(index)=data.x(index) \
                result=p.y(index) RETURN"
            );
            for (label, output) in outputs(&source) {
                for index in [0usize, bound - 1] {
                    let memory = execute(&output, |memory| {
                        memory[0x6000..0x7000].fill(0xA5);
                        memory[0x0600..0x0602].copy_from_slice(&(index as u16).to_le_bytes());
                    });
                    let label = format!("{label}/{ty}/{bound}/{index}");
                    let mut expected = vec![0xA5; 0x1000];
                    for offset in [2 + index * width, 2 + (bound + index) * width] {
                        expected[offset..offset + width]
                            .copy_from_slice(&value.to_le_bytes()[..width]);
                    }
                    assert_bytes(&memory[0x6000..0x7000], &expected, &label);
                    assert_bytes(
                        &memory[0x0602..0x0602 + width],
                        &value.to_le_bytes()[..width],
                        &label,
                    );
                }
            }
        }
    }
}

#[test]
fn embedded_array_execution_record_elements_and_local_arrays() {
    let source = "TYPE Point=[BYTE x INT y] \
        TYPE Scene=[BYTE ARRAY prefix(257) Point ARRAY points(100)] \
        CARD index=$0600 INT result=$0602 BYTE flag=$0604 \
        PROC Main() Scene data Scene POINTER p Point POINTER q \
        p=data p.points(index).x=7 p.points(index).y=-1234 q=p.points \
        result=q(index).y IF p.points(index).y<0 THEN flag=p.points(index).x FI RETURN";
    for (label, output) in outputs(source) {
        for index in [0u16, 1, 85, 99] {
            let memory = execute(&output, |memory| {
                memory[0x0600..0x0602].copy_from_slice(&index.to_le_bytes());
            });
            assert_bytes(
                &memory[0x0602..0x0605],
                &[0x2E, 0xFB, 7],
                &format!("{label}/{index}"),
            );
        }
    }
}

#[test]
fn embedded_array_execution_real_elements() {
    let source = "TYPE Buffer=[BYTE head REAL ARRAY x(100),y(100) BYTE tail] \
        Buffer data=$6001 Buffer POINTER p CARD index=$0600 REAL result=$0602,input=$0610 \
        PROC Main() p=data data.x(index)=input p.y(index)=data.x(index) result=p.y(index) RETURN";
    let value = [0x40, 0x12, 0x34, 0x56, 0x78, 0x90];
    for (label, output) in outputs(source) {
        for index in [0u16, 1, 42, 99] {
            let memory = execute(&output, |memory| {
                memory[0x6000..0x7000].fill(0xA5);
                memory[0x0600..0x0602].copy_from_slice(&index.to_le_bytes());
                memory[0x0610..0x0616].copy_from_slice(&value);
            });
            let label = format!("{label}/{index}");
            let mut expected = vec![0xA5; 0x1000];
            for offset in [2 + usize::from(index) * 6, 602 + usize::from(index) * 6] {
                expected[offset..offset + 6].copy_from_slice(&value);
            }
            assert_bytes(&memory[0x6000..0x7000], &expected, &label);
            assert_bytes(&memory[0x0602..0x0608], &value, &label);
        }
    }
}

#[test]
fn embedded_array_execution_compound_and_argument_order() {
    let source = "TYPE Buffer=[INT ARRAY x(129),y(129)] \
        Buffer first=$6001,second=$6803 Buffer ARRAY rows(3) Buffer POINTER p \
        BYTE calls=$0600 INT result=$0602,combined=$0604 \
        BYTE FUNC Index() calls==+1 p=second RETURN(128) \
        INT FUNC Value() calls==+1 p=second RETURN(4660) \
        BYTE FUNC Row(BYTE r) calls=calls*10+r RETURN(r) \
        PROC Take(INT ARRAY a,b) combined=a(1)+b(2) RETURN \
        PROC Main() calls=0 first.x(128)=3 p=first p.x(Index())==+Value() \
        result=first.x(128) rows(1).x(1)=100 rows(2).y(2)=23 \
        Take(rows(Row(1)).x,rows(Row(2)).y) RETURN";
    for (label, output) in outputs(source) {
        let memory = execute(&output, |_| {});
        assert_eq!(memory[0x0600], 212, "{label}");
        assert_bytes(&memory[0x0602..0x0606], &[0x37, 0x12, 123, 0], &label);
    }
}

#[test]
fn embedded_array_execution_compound_preserves_operand_widths() {
    for (operator, operand, expected) in [
        ("+", 256u16, 13u8),
        ("-", 3, 10),
        ("LSH", 2, 52),
        ("RSH", 2, 3),
    ] {
        let source = format!(
            "TYPE Buffer=[BYTE ARRAY x(2)] Buffer data \
            BYTE calls=$0600,result=$0601 \
            BYTE FUNC Index() calls==+1 RETURN(1) \
            CARD FUNC Value() calls==+1 RETURN({operand}) \
            PROC Main() calls=0 data.x(1)=13 data.x(Index())=={operator} Value() \
            result=data.x(1) RETURN"
        );
        for (label, output) in outputs(&source) {
            let memory = execute(&output, |_| {});
            assert_bytes(
                &memory[0x0600..0x0602],
                &[2, expected],
                &format!("{label}/{operator}"),
            );
        }
    }
}

// These public-language gaps predate inline fields. Keep their reproducers
// executable while slice 4c fixes the shared compound-operation contract.
#[test]
fn characterizes_existing_byte_compound_multiply_nir_gap() {
    let source = "BYTE ARRAY items(2) PROC Main() items(1)==*3 RETURN";
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, SemanticOptions::modern()).unwrap();
    let semir = semantic::ir::lower_program(&ast, &model);
    let nir = crate::nir::lower_program(&semir);
    let errors =
        crate::nir::verify_program(&nir).expect_err("tracked compound multiply typing gap");
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("multiplication must produce cartridge-compatible INT")
    }));
}

#[test]
fn characterizes_existing_byte_compound_division_width_gap() {
    let source = "BYTE ARRAY items(2) BYTE result=$0600 \
        CARD FUNC Divisor() RETURN(256) \
        PROC Main() items(1)=13 items(1)==/Divisor() result=items(1) RETURN";
    for (label, output) in outputs(source) {
        let memory = execute(&output, |_| {});
        // Correct oracle: floor(13 / 256) = 0. MIR currently truncates the
        // divisor to BYTE and returns $FF; this is not a passing semantic case.
        let observed = if label.starts_with("MIR6502/") {
            255
        } else {
            0
        };
        assert_eq!(
            memory[0x0600], observed,
            "tracked compound division gap: {label}"
        );
    }
}

#[test]
fn embedded_array_execution_address_value_in_indirect_destination() {
    let source = "TYPE Buffer=[CARD ARRAY addresses(2) INT ARRAY values(2)] \
        Buffer ARRAY rows(3) Buffer POINTER p INT POINTER q BYTE calls=$0600 INT result=$0602 \
        BYTE FUNC Row() calls==+1 RETURN(1) \
        PROC Main() calls=0 p=@rows(0) rows(1).values(1)=321 \
        p.addresses(1)=CARD(@rows(Row()).values) \
        q=INT POINTER(p.addresses(1)) result=q(1) RETURN";
    for (label, output) in outputs(source) {
        let memory = execute(&output, |_| {});
        assert_eq!(memory[0x0600], 1, "{label}");
        assert_bytes(&memory[0x0602..0x0604], &[0x41, 1], &label);
    }
}
