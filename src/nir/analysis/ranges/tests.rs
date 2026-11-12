use super::*;

fn lower(source: &str) -> NirProgram {
    let tokens = crate::lexer::tokenize(source).unwrap();
    let ast = crate::parser::parse(&tokens).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let program = lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    verify_program(&program).unwrap();
    program
}
fn return_ranges(analysis: &NirRangeAnalysis<'_>, name: &str) -> Vec<(u32, u32)> {
    let routine = analysis
        .program
        .routines
        .iter()
        .find(|r| r.name == name)
        .unwrap();
    routine
        .blocks
        .iter()
        .filter_map(|b| {
            let NirTerminator::Return(Some(value)) = &b.terminator else {
                return None;
            };
            analysis
                .prove_before(routine.id, b.id, b.ops.len(), value)
                .map(|p| (p.range().min(), p.range().max()))
        })
        .collect()
}

#[test]
fn renamed_unaligned_guards_work_with_raw_optimized_lf_and_crlf() {
    let lf = include_str!("../../../../fixtures/runtime/checked_pointer_ranges.act")
        .replace("\r\n", "\n");
    for source in [
        lf.clone(),
        lf.replace('\n', "\r\n")
            .replace("Reserve", "Acquire")
            .replace("StopRange", "HaltHere"),
    ] {
        let raw = lower(&source);
        let optimized = optimize_program(&raw).unwrap();
        for program in [&raw, &optimized] {
            let before = program.clone();
            let analysis = analyze_unsigned_ranges(program).unwrap();
            let stop = program
                .routines
                .iter()
                .find(|r| r.name == "StopRange" || r.name == "HaltHere")
                .unwrap();
            assert!(analysis.is_non_returning(stop.id));
            let reserve = if source.contains("Acquire") {
                "Acquire"
            } else {
                "Reserve"
            };
            assert_eq!(return_ranges(&analysis, reserve), [(0x2103, 0x270B)]);
            assert_eq!(return_ranges(&analysis, "Unchecked"), [(0, 65535)]);
            for name in ["Guarded", "Unrestricted"] {
                let r = program.routines.iter().find(|r| r.name == name).unwrap();
                let (block, index, args) = r
                    .blocks
                    .iter()
                    .find_map(|b| {
                        b.ops.iter().enumerate().find_map(|(i, op)| {
                            if let NirOp::Call {
                                callee: NirCallee::User { name, .. },
                                args,
                                ..
                            } = op
                            {
                                if name == "CopyBytes" {
                                    return Some((b, i, args));
                                }
                            }
                            None
                        })
                    })
                    .unwrap();
                let bounds = args
                    .iter()
                    .take(2)
                    .map(|v| {
                        analysis
                            .prove_before(r.id, block.id, index, v)
                            .unwrap()
                            .range()
                    })
                    .map(|r| (r.min(), r.max()))
                    .collect::<Vec<_>>();
                assert_eq!(
                    bounds,
                    if name == "Guarded" {
                        vec![(0x2104, 0x280B), (0x2104, 0x270C)]
                    } else {
                        vec![(0, 65535); 2]
                    }
                );
            }
            assert_eq!(program, &before, "analysis changed executable NIR");
        }
    }
}

#[test]
fn returning_handlers_and_changed_definitions_do_not_supply_bounds() {
    let source = "BYTE fault\nCARD cursor\nPROC Die(BYTE r) fault=r DO OD RETURN\n\
        CARD FUNC Checked(CARD n) CARD c,req c=cursor req=n\n\
        IF c<1000 THEN Die(1) FI IF c>2000 THEN Die(1) FI\n\
        IF req>2000-c THEN Die(2) FI\nRETURN(c+req)\n\
        PROC Main() RETURN\n";
    let p = lower(source);
    assert_eq!(
        return_ranges(&analyze_unsigned_ranges(&p).unwrap(), "Checked"),
        [(1000, 2000)]
    );
    for (altered, expected) in [
        (source.replace("DO OD RETURN", "RETURN"), (0, 65535)),
        (
            source.replace("RETURN(c+req)", "req=n+1 RETURN(c+req)"),
            (1001, 3001),
        ),
        (
            source.replace("RETURN(c+req)", "c=0 RETURN(c+req)"),
            (0, 1000),
        ),
        (
            source
                .replace("CARD cursor\n", "CARD cursor=$E8\n")
                .replace("RETURN(c+req)", "c=cursor RETURN(c+req)"),
            (0, 65535),
        ),
    ] {
        let p = lower(&altered);
        assert_eq!(
            return_ranges(&analyze_unsigned_ranges(&p).unwrap(), "Checked"),
            [expected]
        );
    }
}

#[test]
fn wrap_casts_signed_values_and_joins_are_conservative() {
    let p = lower(
        "CARD out\nPROC Die() DO OD RETURN\n\
        CARD FUNC Wide(CARD p) IF p<1000 THEN Die() FI IF p>2000 THEN Die() FI RETURN(p+100)\n\
        CARD FUNC Wrap(CARD p) IF p<65000 THEN Die() FI RETURN(p+1000)\n\
        BYTE FUNC Narrow(CARD p) IF p>255 THEN Die() FI RETURN(BYTE(p))\n\
        BYTE FUNC Truncate(CARD p) RETURN(BYTE(p))\n\
        CARD FUNC Signed(INT p) RETURN(CARD(p))\n\
        CARD FUNC Joined(BYTE choose) CARD p IF choose THEN p=1000 ELSE p=2000 FI RETURN(p)\n\
        PROC Main() RETURN\n",
    );
    let a = analyze_unsigned_ranges(&p).unwrap();
    assert_eq!(return_ranges(&a, "Wide"), [(1100, 2100)]);
    assert_eq!(return_ranges(&a, "Wrap"), [(0, 65535)]);
    assert_eq!(return_ranges(&a, "Narrow"), [(0, 255)]);
    assert_eq!(return_ranges(&a, "Truncate"), [(0, 255)]);
    assert_eq!(return_ranges(&a, "Signed"), [(0, 65535)]);
    assert_eq!(return_ranges(&a, "Joined"), [(1000, 2000)]);
}

#[test]
fn pointer_writes_and_machine_calls_invalidate_stored_ranges() {
    for barrier in ["writeptr^=1", "Opaque()", "p=raw"] {
        let source = format!(
            "CARD raw\nBYTE POINTER writeptr\nPROC Die() DO OD RETURN\nPROC Opaque()
ASM OPAQUE
nop
ENDASM
RETURN\n\
            CARD FUNC Test(CARD n) CARD p p=n IF p<1000 THEN Die() FI IF p>2000 THEN Die() FI\n\
            {barrier} RETURN(p)\nPROC Main() RETURN\n"
        );
        let p = lower(&source);
        assert_eq!(
            return_ranges(&analyze_unsigned_ranges(&p).unwrap(), "Test"),
            [(0, 65535)]
        );
    }
}

#[test]
fn loop_backedges_cannot_create_entry_ranges_and_limits_fail_closed() {
    let p = lower(
        "PROC Die() DO OD RETURN\nCARD FUNC Loop(CARD n)\n\
        WHILE n<1000 DO n==+1 OD RETURN(n)\nPROC Main() RETURN\n",
    );
    let a = analyze_unsigned_ranges(&p).unwrap();
    // Either a finite overapproximation or an exhausted analysis is valid.
    for (lo, hi) in return_ranges(&a, "Loop") {
        assert!(lo <= 1000 && hi == 65535);
    }
    let mut large = lower("PROC Main() RETURN\n");
    let r = &mut large.routines[0];
    r.blocks = (0..=MAX_BLOCKS)
        .map(|i| NirBlock {
            id: BlockId(i as u32),
            label: format!("b{i}"),
            params: vec![],
            ops: vec![],
            terminator: if i == MAX_BLOCKS {
                NirTerminator::Return(None)
            } else {
                NirTerminator::Goto(NirEdge {
                    target: BlockId(i as u32 + 1),
                    args: vec![],
                })
            },
        })
        .collect();
    let a = analyze_unsigned_ranges(&large).unwrap();
    assert!(a.evaluations(large.routines[0].id).is_none());
    assert!(!a.is_non_returning(large.routines[0].id));
}

#[test]
fn non_return_proofs_do_not_assume_names_recursion_or_machine_control_flow() {
    let p = lower(
        "BYTE patch=$2C00\nBYTE POINTER raw\nPROC WritesCode() patch=96 DO OD RETURN\nPROC WritesPointer() raw^=96 DO OD RETURN\nPROC Error() RETURN\nPROC Cycle() Cycle() RETURN\n\
        PROC Machine()
ASM OPAQUE
rts
ENDASM
DO OD RETURN\n\
        PROC Sink() DO OD RETURN\nPROC Through() Sink() RETURN\nPROC Main() RETURN\n",
    );
    let a = analyze_unsigned_ranges(&p).unwrap();
    for r in &p.routines {
        assert_eq!(
            a.is_non_returning(r.id),
            matches!(r.name.as_str(), "Sink" | "Through"),
            "{}",
            r.name
        );
    }
}

#[test]
fn queries_reject_unavailable_or_mistyped_temps() {
    let p = lower("CARD FUNC Read(CARD n) RETURN(n)\nPROC Main() RETURN\n");
    let a = analyze_unsigned_ranges(&p).unwrap();
    let r = p.routines.iter().find(|r| r.name == "Read").unwrap();
    let b = &r.blocks[0];
    let NirTerminator::Return(Some(value)) = &b.terminator else {
        panic!()
    };
    assert!(a.prove_before(r.id, b.id, 0, value).is_none());
    assert!(a.prove_before(r.id, b.id, b.ops.len(), value).is_some());
    let mut wrong = value.clone();
    if let NirValue::Temp { ty, .. } = &mut wrong {
        ty.kind = NirTypeKind::U8;
        ty.width = Some(ByteSize::ONE);
    }
    assert!(a.prove_before(r.id, b.id, b.ops.len(), &wrong).is_none());
}

#[test]
fn transfer_budget_discards_incomplete_loop_facts() {
    let p = lower(
        "CARD FUNC Slow() CARD n n=0 WHILE n<60000 DO n==+1 OD RETURN(n)\nPROC Main() RETURN\n",
    );
    let a = analyze_unsigned_ranges(&p).unwrap();
    let r = p.routines.iter().find(|r| r.name == "Slow").unwrap();
    assert!(a.evaluations(r.id).is_none());
    assert!(return_ranges(&a, "Slow").is_empty());
}

fn summary(a: &NirRangeAnalysis<'_>, name: &str) -> Option<(u32, u32)> {
    let id = a
        .program
        .routines
        .iter()
        .find(|r| r.name == name)
        .unwrap()
        .id;
    a.normal_return_range(id).map(|r| (r.min(), r.max()))
}

#[test]
fn universal_returns_reach_complete_stores_and_call_arguments() {
    let lf = include_str!("../../../../fixtures/runtime/checked_pointer_ranges.act")
        .replace("\r\n", "\n");
    for source in [lf.clone(), lf.replace('\n', "\r\n")] {
        let raw = lower(&source);
        let opt = optimize_program(&raw).unwrap();
        for p in [&raw, &opt] {
            let a = analyze_unsigned_ranges(p).unwrap();
            assert_eq!(summary(&a, "Reserve"), Some((0x2103, 0x270B)));
            // Zero-size allocation remains part of the generic postcondition.
            // A constant nonzero caller request does not silently imply extent.
            let r = p.routines.iter().find(|r| r.name == "FromReturn").unwrap();
            let (block, index, args) = r
                .blocks
                .iter()
                .find_map(|b| {
                    b.ops.iter().enumerate().find_map(|(i, op)| {
                        if let NirOp::Call {
                            callee: NirCallee::User { name, .. },
                            args,
                            ..
                        } = op
                        {
                            if name == "CopyBytes" {
                                return Some((b, i, args));
                            }
                        }
                        None
                    })
                })
                .unwrap();
            let ranges = args
                .iter()
                .take(2)
                .map(|v| a.prove_before(r.id, block.id, index, v).unwrap().range())
                .map(|r| (r.min(), r.max()))
                .collect::<Vec<_>>();
            assert_eq!(ranges, [(0x2104, 0x280B), (0x2104, 0x270C)]);
        }
    }
}

#[test]
fn caller_and_callee_parameter_and_local_ids_are_distinct() {
    let p = lower(
        "PROC Touch(CARD n) CARD saved saved=n RETURN\n\
        CARD FUNC Keep(CARD n) CARD saved saved=1500 n=1600 Touch(1) RETURN(saved+n)\nPROC Main() RETURN\n",
    );
    let a = analyze_unsigned_ranges(&p).unwrap();
    assert_eq!(summary(&a, "Keep"), Some((3100, 3100)));
}

#[test]
fn terminal_failure_writes_are_excluded_only_from_normal_continuations() {
    let source = "CARD held,cursor=$E8\nPROC Stop() DO OD RETURN\n\
        PROC Rewind(CARD mark) IF mark<1000 THEN held=0 Stop() FI cursor=mark RETURN\n\
        CARD FUNC Across() held=1500 Rewind(1000) RETURN(held)\nPROC Main() RETURN\n";
    let p = lower(source);
    let a = analyze_unsigned_ranges(&p).unwrap();
    assert_eq!(summary(&a, "Across"), Some((1500, 1500)));
    assert!(format_program(&p).contains("call Stop"));
    let changed = lower(&source.replace("DO OD RETURN", "RETURN"));
    assert_eq!(
        summary(&analyze_unsigned_ranges(&changed).unwrap(), "Across"),
        Some((0, 65535))
    );
}

#[test]
fn mutable_globals_require_reaching_stores_on_every_path() {
    let p = lower(
        "CARD saved,cursor=$E8\nPROC Stop() DO OD RETURN\n\
        CARD FUNC Take(CARD n) CARD p p=cursor IF p<1000 THEN Stop() FI IF p>2000 THEN Stop() FI\n\
        IF n>2000-p THEN Stop() FI cursor=p+n RETURN(p)\n\
        PROC Rewind(CARD mark) cursor=mark RETURN\n\
        CARD FUNC Across() saved=Take(0) Rewind(1000) RETURN(saved)\n\
        CARD FUNC Maybe(BYTE first) saved=0 IF first THEN saved=Take(0) FI RETURN(saved)\n\
        CARD FUNC Read() RETURN(saved)\nPROC Main() RETURN\n",
    );
    let a = analyze_unsigned_ranges(&p).unwrap();
    assert_eq!(summary(&a, "Across"), Some((1000, 2000)));
    assert_eq!(summary(&a, "Maybe"), Some((0, 2000)));
    assert_eq!(summary(&a, "Read"), Some((0, 65535)));
    let r = p.routines.iter().find(|r| r.name == "Across").unwrap();
    assert!(a.storage_ranges_at_entry(r.id, r.blocks[0].id).is_empty());
}

#[test]
fn opaque_bodies_recursion_retargeting_and_long_dependencies_have_no_summary() {
    let p = lower(
        "CARD FUNC External=$1234()\nCARD FUNC Rec() RETURN(Rec())\n\
        CARD FUNC Early()\nASM OPAQUE\nlda #1\nrts\nENDASM\nRETURN(1500)\n\
        CARD FUNC Caller() RETURN(External())\nPROC Main() RETURN\n",
    );
    let a = analyze_unsigned_ranges(&p).unwrap();
    for name in ["External", "Rec", "Early", "Caller"] {
        assert_eq!(summary(&a, name), None, "{name}");
    }
    let mut source = "CARD FUNC Leaf() RETURN(7)\n".to_string();
    let mut previous = "Leaf".to_string();
    for i in 0..17 {
        source.push_str(&format!("CARD FUNC Link{i}() RETURN({previous}())\n"));
        previous = format!("Link{i}");
    }
    source.push_str("PROC Main() RETURN\n");
    let p = lower(&source);
    let a = analyze_unsigned_ranges(&p).unwrap();
    assert_eq!(summary(&a, "Link14"), Some((7, 7)));
    assert_eq!(summary(&a, "Link15"), None);
    assert_eq!(summary(&a, "Link16"), None);
}

#[test]
fn point_storage_queries_replay_writes_instead_of_reusing_entry_facts() {
    let program = lower("MODULE\nPROC Main()\n CARD cell\n cell=$2103\n cell=$270B\nRETURN\n");
    let r = &program.routines[0];
    let analysis = analyze_unsigned_ranges(&program).unwrap();
    let b = &r.blocks[0];
    let mut observed = Vec::new();
    for i in 0..=b.ops.len() {
        for p in analysis.storage_ranges_before(r.id, b.id, i) {
            assert_eq!(p.source().2, i);
            observed.push((p.range().min(), p.range().max()));
        }
    }
    assert!(observed.contains(&(0x2103, 0x2103)));
    assert_eq!(observed.last(), Some(&(0x270B, 0x270B)));
    assert!(
        analysis
            .storage_ranges_before(r.id, b.id, b.ops.len() + 1)
            .is_empty()
    );
}
