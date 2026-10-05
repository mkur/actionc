use actionc::{
    lexer,
    mir65816::{
        self, Mir65816Op, Mir65816Value,
        analysis::{QueryError, RoutineAnalysis},
        emit, image,
    },
    nir, parser, semantic,
    target::TargetId,
};

fn lower(source: &str, optimize: bool) -> mir65816::Mir65816Program {
    let ast = parser::parse(&lexer::tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(
        &ast,
        semantic::SemanticOptions::modern().with_target(TargetId::Wdc65816Native),
    )
    .unwrap();
    let nir = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    let nir = if optimize {
        nir::optimize_program_with_promotion(&nir, nir::NirPromotionPolicy::Native65816).unwrap()
    } else {
        nir
    };
    mir65816::lower_program(&nir).unwrap()
}

#[test]
fn mixed_record_program_checks_all_live_uses_in_raw_and_optimized_lf_and_crlf() {
    let lf =
        include_str!("../tools/compare65816/record_probe/record_flow.act").replace("\r\n", "\n");
    let crlf = lf.replace('\n', "\r\n");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "actionc-logical-record-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir(&directory).unwrap();
    for optimize in [false, true] {
        let mut images = vec![];
        for (name, text, newline) in [("lf", &lf, "\n"), ("crlf", &crlf, "\r\n")] {
            let input = directory.join(format!("{name}-{optimize}"));
            std::fs::create_dir(&input).unwrap();
            std::fs::write(input.join("recordprobe.act"), text).unwrap();
            std::fs::write(
                input.join("main.act"),
                [
                    "MODULE ANALYSIS",
                    "USE RECORDPROBE",
                    "PROC Main() RETURN",
                    "ENDMODULE",
                    "",
                ]
                .join(newline),
            )
            .unwrap();
            let modules = actionc::includes::ModuleLoadOptions {
                module_paths: vec![input.clone()],
                ..Default::default()
            };
            let prepared = actionc::compiler::native65816::prepare_file(
                input.join("main.act"),
                optimize,
                &modules,
            )
            .unwrap();
            let program = prepared.mir;
            let machine = emit::materialize(&program).unwrap();
            for r in &machine.prepared.routines {
                if r.entry.external || r.helper.is_some() {
                    continue;
                }
                let a = RoutineAnalysis::new(r).unwrap();
                for (id, _) in &r.temps {
                    let temp = a.temp(*id).unwrap();
                    for u in &a.value(temp).unwrap().uses {
                        let point = a
                            .point(a.block(u.point.block).unwrap(), u.point.index)
                            .unwrap();
                        assert!(matches!(
                            a.available(point, temp),
                            Ok(true) | Err(QueryError::Unreachable)
                        ));
                    }
                }
            }
            let options = image::LinkOptions {
                stack_checks: true,
                code_origin: 0x10000,
                data_origin: 0x180000,
                read_only_origin: None,
                zero_fill_origin: None,
                stack_overflow: 0x48000,
                arithmetic_fault: None,
                nmi_extra_stack: 0,
                imports: vec![],
            };
            images.push(
                image::link(&program, &machine, &options)
                    .unwrap()
                    .to_json()
                    .unwrap(),
            );
        }
        assert_eq!(
            images[0], images[1],
            "LF/CRLF artifacts differ; optimize={optimize}"
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn emission_rejects_forged_use_before_definition_before_placement() {
    let mut program = lower(
        "CARD FUNC Work(CARD n) RETURN(n+1) PROC Main() RETURN",
        false,
    );
    let r = &mut program.routines[0];
    let (source, ty) = r.temps[0].clone();
    let width = ty.width.unwrap();
    let forged = nir::TempId(1000);
    r.temps.push((forged, ty));
    r.blocks[0].ops.insert(
        0,
        Mir65816Op::Cast {
            dest: forged,
            from: width,
            from_signed: false,
            to: width,
            kind: nir::NirCastKind::Integer,
            value: Mir65816Value::Temp(source, width),
        },
    );
    // The old MIR shape/ABI verifier accepts the complete typed definitions.
    mir65816::verify_program(&program).unwrap();
    let error = emit::materialize(&program).unwrap_err();
    assert!(
        error.contains("invalid logical MIR") && error.contains("not definitely defined"),
        "{error}"
    );
}
