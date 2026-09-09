use actionc::{
    lexer::tokenize,
    nir,
    parser::parse,
    semantic::{self, SemanticOptions},
    target::{Endian, TargetId},
};

fn lower(source: &str, target: TargetId) -> nir::NirProgram {
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let mut options = SemanticOptions::modern().with_target(target);
    options.algebraic_types.unions = true;
    let model = semantic::analyze_with_options(&ast, options).unwrap();
    let raw = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    nir::verify_program(&raw).unwrap();
    raw
}

#[test]
fn native_union_pointer_views_preserve_width_endianness_and_byte_displacements() {
    for (target, literal, pointer_width, bytes) in [
        (TargetId::Wdc65816Small, "$5678", 2, vec![0x78, 0x56]),
        (
            TargetId::Wdc65816Native,
            "$345678",
            3,
            vec![0x78, 0x56, 0x34],
        ),
        (
            TargetId::Motorola68000,
            "$12345678",
            4,
            vec![0x12, 0x34, 0x56, 0x78],
        ),
    ] {
        let source = format!(
            "TYPE Pair=[BYTE low,high] TYPE View=UNION [BYTE POINTER link CARD word BYTE ARRAY bytes(4) Pair parts] \
            View original BYTE POINTER seed=[{literal}] BYTE result \
            PROC Main() original.link=seed result=original.bytes(0) original.parts.high=7 \
            LET saved=original result=saved.bytes(1) RETURN"
        );
        let raw = lower(&source, target);
        for nir in [raw.clone(), nir::optimize_program(&raw).unwrap()] {
            nir::verify_program(&nir).unwrap();
            if target == TargetId::Motorola68000 {
                use actionc::mir68k::*;
                let mir = lower_program(&nir).unwrap();
                assert_eq!(mir.endian, Endian::Big);
                assert_eq!(mir.data_pointer_width.get(), pointer_width);
                assert_eq!(
                    mir.data.iter().find(|d| d.name == "seed").unwrap().bytes,
                    bytes
                );
                let ops: Vec<_> = mir
                    .routines
                    .iter()
                    .flat_map(|r| &r.blocks)
                    .flat_map(|b| &b.ops)
                    .collect();
                assert!(ops.iter().any(|op|matches!(op,Mir68kOp::Store{address,width,..} if width.get()==pointer_width && address.displacement.get()==0)));
                assert!(ops.iter().any(|op|matches!(op,Mir68kOp::Store{address,width,..} if width.get()==1 && address.displacement.get()==1)));
                assert!(ops.iter().any(
                    |op| matches!(op,Mir68kOp::Copy{bytes,overlap_safe:true,..} if bytes.get()==4)
                ));
            } else {
                use actionc::mir65816::*;
                let mir = lower_program(&nir).unwrap();
                assert_eq!(mir.endian, Endian::Little);
                assert_eq!(mir.data_pointer_width.get(), pointer_width);
                assert_eq!(
                    mir.data.iter().find(|d| d.name == "seed").unwrap().bytes,
                    bytes
                );
                let ops: Vec<_> = mir
                    .routines
                    .iter()
                    .flat_map(|r| &r.blocks)
                    .flat_map(|b| &b.ops)
                    .collect();
                assert!(ops.iter().any(|op|matches!(op,Mir65816Op::Store{address,width,..} if width.get()==pointer_width && address.displacement.get()==0)));
                assert!(ops.iter().any(|op|matches!(op,Mir65816Op::Store{address,width,..} if width.get()==1 && address.displacement.get()==1)));
                assert!(ops.iter().any(
                    |op| matches!(op,Mir65816Op::Copy{bytes,overlap_safe:true,..} if bytes.get()==4)
                ));
            }
        }
    }
}

#[test]
fn union_payloads_do_not_bypass_native_variant_error_adapter_limits() {
    let source = "TYPE View=UNION [CARD word] TYPE Event=VARIANT [DATA [View value]] \
        Event current CARD result PROC Main()\nCASE current OF\nWHEN Event.DATA(saved) THEN\nresult=saved.word\nESAC\nRETURN";
    for target in [
        TargetId::Motorola68000,
        TargetId::Wdc65816Small,
        TargetId::Wdc65816Native,
    ] {
        let raw = lower(source, target);
        for nir in [raw.clone(), nir::optimize_program(&raw).unwrap()] {
            let errors = if target == TargetId::Motorola68000 {
                format!("{:?}", actionc::mir68k::lower_program(&nir).unwrap_err())
            } else {
                format!("{:?}", actionc::mir65816::lower_program(&nir).unwrap_err())
            };
            assert!(
                errors.contains("native target Error adapter"),
                "{target:?}: {errors}"
            );
        }
    }
}
