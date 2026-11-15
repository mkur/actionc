use super::*;

fn moving() -> MirMem {
    MirMem::Spill {
        id: MirSpillId(2),
        offset: 0,
    }
}
fn fixture() -> MirProgram {
    let mut p = oracle_program();
    p.routines[0].frame.spills = (0..3).map(MirSpillId).collect();
    let ops = &mut p.routines[0].blocks[0].ops;
    let end = ops.len() - 1;
    ops.insert(
        end,
        MirOp::Store {
            dst: MirAddr::Direct(moving()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
    );
    ops.insert(end + 1, load(moving()));
    p
}
fn selected(p: &MirProgram, origin: u16, runtime: crate::runtime::Runtime) -> MirProgram {
    select(
        p.clone(),
        &[p],
        &clean(p),
        None,
        &Mir6502Config::optimized(),
        origin,
        runtime,
        &Trace::disabled(),
    )
    .unwrap()
}

#[test]
fn private_compaction_executes_every_byte_and_preserves_addresses_outside_the_home() {
    for runtime in [
        crate::runtime::Runtime::ActionCart,
        crate::runtime::Runtime::Standalone,
    ] {
        for origin in [0x3000, 0x41c7] {
            let p = fixture();
            let after = selected(&p, origin, runtime);
            assert_eq!(after.routines[0].frame.spills, vec![MirSpillId(2)]);
            assert_eq!(after.routines[0].blocks, p.routines[0].blocks);
            let a = emit_output(&p, origin, runtime);
            let b = emit_output(&after, origin, runtime);
            assert_eq!(
                crate::codegen::format_load_file(&a).len()
                    - crate::codegen::format_load_file(&b).len(),
                2
            );
            for output in [&a, &b] {
                for byte in 0..=255 {
                    let mut ram = [0u8; 65536];
                    let start = usize::from(origin);
                    ram[start..start + output.bytes.len()].copy_from_slice(&output.bytes);
                    ram[0x600] = byte;
                    ram[0x5ff] = 0x5a;
                    ram[0x602] = 0xc3;
                    crate::mir6502::materialize::leaf_test_cpu::run_memory(
                        &mut ram,
                        usize::from(output.routine_addresses[0].address),
                    );
                    assert_eq!(
                        (ram[0x5ff], ram[0x600], ram[0x601], ram[0x602]),
                        (0x5a, byte, byte ^ 0xa5, 0xc3)
                    );
                }
            }
        }
    }
}

#[test]
fn exposed_wide_offset_and_non_native_uses_keep_the_moving_home_fixed() {
    let p = fixture();
    let mut alternatives = Vec::new();
    let mut address = p.clone();
    address.routines[0].blocks[0].ops.push(MirOp::LeaAddr {
        dst: MirDef::Reg(MirReg::A),
        target: moving(),
        width: MirWidth::Word,
    });
    alternatives.push(address);
    let mut wide = p.clone();
    wide.routines[0].blocks[0].ops.push(MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(moving()),
        width: MirWidth::Word,
    });
    alternatives.push(wide);
    let mut offset = p.clone();
    offset.routines[0].blocks[0].ops.push(load(MirMem::Spill {
        id: MirSpillId(2),
        offset: 1,
    }));
    alternatives.push(offset);
    let mut non_native = p.clone();
    non_native.routines[0].blocks[0].ops.push(MirOp::Move {
        dst: MirDef::Reg(MirReg::A),
        src: MirValue::PointerCell(moving()),
        width: MirWidth::Byte,
    });
    alternatives.push(non_native);
    // Historical observations remain relevant even when current operations are simple.
    for historical in alternatives {
        let session = Mir6502Trace::default();
        let after = select(
            p.clone(),
            &[&historical],
            &clean(&p),
            None,
            &Mir6502Config::optimized(),
            0x3000,
            crate::runtime::Runtime::ActionCart,
            &session.context(),
        )
        .unwrap();
        assert_eq!(after, p);
        assert_eq!(result(&session.events())["reason"], "CoreAllocation");
        assert!(session.manifest()["work"]["emission_invocations"].is_null());
    }
}

#[test]
fn explicit_source_and_numeric_aliases_keep_the_live_byte_at_its_old_address() {
    for source in [false, true] {
        let mut p = fixture();
        if source {
            p.globals.push(MirGlobal {
                id: crate::nir::SymbolId(0),
                name: "fixed_alias".into(),
                kind: "BYTE".into(),
                width: Some(MirWidth::Byte),
                storage_size: 1,
                backing: MirGlobalBacking::Absolute(0x3002),
                init: None,
            });
        } else {
            p.routines[0].blocks[0]
                .ops
                .push(load(MirMem::Absolute(0x3002)));
        }
        let (after, work, events) = run(&p, &clean(&p));
        assert_eq!(after, p);
        assert_eq!(result(&events)["reason"], "PrivateMovement");
        assert_eq!(work["work"]["emission_invocations"], 1);
    }
}

#[test]
fn historical_physical_alias_cannot_become_a_retired_private_capture() {
    let p = fixture();
    let mut historical = p.clone();
    historical.routines[0].blocks[0]
        .ops
        .push(load(MirMem::Absolute(0x3001)));
    let session = Mir6502Trace::default();
    let after = select(
        p.clone(),
        &[&historical],
        &clean(&p),
        None,
        &Mir6502Config::optimized(),
        0x3000,
        crate::runtime::Runtime::ActionCart,
        &session.context(),
    )
    .unwrap();
    assert_eq!(after, p);
    // Historical exposure now rejects allocation removal before movement replay.
    assert_eq!(result(&session.events())["reason"], "ObservableAllocation");
    assert_eq!(session.manifest()["work"]["emission_invocations"], 1);
}

#[test]
fn moving_a_private_byte_still_requires_earlier_decision_replay() {
    let p = fixture();
    let mut log = Preservation::new(true);
    let mut before = p.clone();
    before.routines[0].blocks[0].ops.insert(0, imm(99));
    log.select(Family::CheckedRanges, before, |_| Ok(p.clone()))
        .unwrap();
    let (after, work, events) = run(&p, &log);
    assert_eq!(after, p);
    assert_eq!(result(&events)["reason"], "EarlierDecision(CheckedRanges)");
    assert_eq!(work["work"]["emission_invocations"], 2);
}

#[test]
fn zero_page_and_rom_boundary_crossings_do_not_gain_compaction_permission() {
    let p = fixture();
    let removed = discover(&p, &[]).0;
    let after = erase(&p, &removed);
    for origin in [0xfe, 0x3fe, 0x7ffe, 0x8000] {
        assert!(
            PrivateByteMove::discover(
                &p,
                &[],
                &MaterializeLayout::new(&p, origin),
                &MaterializeLayout::new(&after, origin),
                &removed
            )
            .is_none()
        );
    }
}

#[test]
#[ignore = "requires MADS assembler on PATH or ACTIONC_MADS"]
fn private_compaction_listings_reconstruct_and_relocate_with_lf_and_crlf() {
    let dir = std::env::temp_dir().join(format!("actionc-private-move-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for runtime in [
        crate::runtime::Runtime::ActionCart,
        crate::runtime::Runtime::Standalone,
    ] {
        let p = fixture();
        let outputs = [0x3000, 0x41c7]
            .map(|origin| emit_output(&selected(&p, origin, runtime), origin, runtime));
        for (index, origin) in [0x3000, 0x41c7].into_iter().enumerate() {
            let lf = crate::compiler::artifacts::format_listing_with_source(&outputs[0], "")
                .replace("\r\n", "\n")
                .replace(
                    "ACTIONC_ORIGIN = $3000",
                    &format!("ACTIONC_ORIGIN = ${origin:04X}"),
                );
            for crlf in [false, true] {
                let asm = dir.join(format!("{runtime:?}-{origin:x}-{crlf}.asm"));
                let xex = asm.with_extension("xex");
                std::fs::write(
                    &asm,
                    if crlf {
                        lf.replace('\n', "\r\n")
                    } else {
                        lf.clone()
                    },
                )
                .unwrap();
                let result = std::process::Command::new(
                    std::env::var_os("ACTIONC_MADS").unwrap_or_else(|| "mads".into()),
                )
                .arg(&asm)
                .arg(format!("-o:{}", xex.display()))
                .arg("-s")
                .output()
                .unwrap();
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stdout)
                );
                assert_eq!(
                    std::fs::read(xex).unwrap(),
                    crate::codegen::format_load_file(&outputs[index])
                );
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
