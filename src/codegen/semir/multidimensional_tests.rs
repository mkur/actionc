use super::array_execution_tests::{execute, outputs_with_options};
use crate::semantic::SemanticOptions;

fn outputs(source: &str) -> Vec<(String, crate::codegen::CodegenOutput)> {
    outputs_with_options(
        source,
        SemanticOptions {
            multidimensional_arrays: true,
            ..SemanticOptions::modern()
        },
    )
}

#[test]
fn multidimensional_execution_rectangles_rank_three_and_widened_byte_coordinates() {
    let source = "BYTE row,column,depth CARD ARRAY grid(3,129) BYTE ARRAY volume(2,3,5) CARD observed=$0600,neighbor=$0602 CARD POINTER flat BYTE POINTER bytes BYTE voxel=$0606,otherVoxel=$0607 \
        PROC Main() row=2 column=128 grid(row,column)=$1234 grid(row,column)==+1 \
        flat=grid observed=flat(386) neighbor=flat(130) volume(0,2,4)=55 volume(1,2,4)=77 bytes=volume voxel=bytes(29) otherVoxel=bytes(14) RETURN";
    for (label, output) in outputs(source) {
        let memory = execute(&output, |_| {});
        assert_eq!(&memory[0x0600..0x0604], &[0x35, 0x12, 0, 0], "{label}");
        assert_eq!(&memory[0x0606..0x0608], &[77, 55], "{label}");
    }
}

#[test]
fn multidimensional_execution_inline_fields_capture_coordinates_and_compound_rhs() {
    let source = "TYPE Tile=[BYTE tag CARD ARRAY pixels(2,129)] Tile first=$5001,second=$5801 Tile POINTER p \
        BYTE calls=$0600 CARD observed=$0602,guard=$0604 \
        BYTE FUNC Row() calls==+1 p=second RETURN(1) \
        BYTE FUNC Column() calls==+10 RETURN(128) \
        CARD FUNC Value() calls==+100 first.pixels(1,128)=40 RETURN(2) \
        PROC Main() p=first calls=0 first.pixels(1,128)=3 second.pixels(1,128)=10 \
        p.pixels(Row(),Column())==+Value() observed=first.pixels(1,128) guard=second.pixels(1,128) RETURN";
    for (label, output) in outputs(source) {
        let memory = execute(&output, |m| m[0x5000..0x5B00].fill(0xA5));
        assert_eq!(memory[0x0600], 111, "{label}");
        assert_eq!(&memory[0x0602..0x0606], &[42, 0, 10, 0], "{label}");
        assert_eq!(memory[0x5001], 0xA5, "{label}");
        assert_eq!(memory[0x5801], 0xA5, "{label}");
    }
}

#[test]
fn multidimensional_execution_partials_zero_fill_and_static_addresses() {
    let source = "CARD ARRAY grid(2,3)=[1 2 3 4] CARD POINTER p=[@grid(1,2)] \
        CARD observed=$0600,tail=$0602,localTail=$0604 \
        PROC Main() CARD ARRAY local(2,3)=[9] observed=grid(1,0) tail=p^ localTail=local(1,2) RETURN";
    for (label, output) in outputs(source) {
        let memory = execute(&output, |_| {});
        assert_eq!(&memory[0x0600..0x0606], &[4, 0, 0, 0, 0, 0], "{label}");
    }
}

#[test]
fn multidimensional_execution_descriptor_rebinding_preserves_declared_shape() {
    let source = "CARD ARRAY grid(2,3)=[1 2 3 4 5 6],other(3,2)=[10 20 30 40 50 60] \
        CARD observed=$0600,updated=$0602 CARD POINTER p \
        BYTE FUNC Row() grid=other RETURN(1) \
        PROC Main() observed=grid(Row(),2) p=other grid=p grid(1,2)=99 updated=other(2,1) RETURN";
    for (label, output) in outputs(source) {
        let memory = execute(&output, |_| {});
        assert_eq!(&memory[0x0600..0x0604], &[6, 0, 99, 0], "{label}");
    }
}

#[test]
fn multidimensional_execution_mutable_byte_wide_and_record_arrays() {
    let source = "TYPE Item=[BYTE tag CARD number] Item ARRAY items(2,3),replacement(3,2) \
        BYTE ARRAY bytes(2,3)=[7],newBytes(3,2)=[9] LONGINT ARRAY wide(2,3)=[-7],newWide(3,2)=[-9] \
        CARD result=$0600 BYTE zero=$0602 LONGINT value=$0604 \
        PROC Main() BYTE ARRAY scratch(2,3)=[11],localOther(3,2)=[13] \
        zero=bytes(1,2)+scratch(1,2) bytes=newBytes bytes(1,2)=21 \
        scratch=localOther scratch(1,2)=22 items=replacement items(1,2).number=1234 \
        result=replacement(2,1).number+newBytes(2,1)+localOther(2,1) \
        wide=newWide wide(1,2)=-123456 value=newWide(2,1) RETURN";
    for (label, output) in outputs(source) {
        let memory = execute(&output, |_| {});
        assert_eq!(&memory[0x0600..0x0603], &[0xFD, 4, 0], "{label}");
        assert_eq!(
            &memory[0x0604..0x0608],
            &(-123456i32).to_le_bytes(),
            "{label}"
        );
    }
}
