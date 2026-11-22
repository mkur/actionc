use super::*;
use super::library::{graphics_cio_boundary, reviewed_cio_transfer};
use crate::mir6502::analysis::known_callees::{MirAcyclicEntryAnalysis, MirCalleeEntryStage, MirKnownCalleeSummaries, MirKnownMemoryReads, MirKnownMemoryWrites};
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);

fn checked_nir(source: &str) -> crate::nir::NirProgram {
    let path = std::env::temp_dir().join(format!("actionc-contract-{}-{}.act", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&path, source).unwrap();
    let loaded = crate::includes::load_compilation(&path, &crate::includes::ModuleLoadOptions::default()).unwrap();
    std::fs::remove_file(path).unwrap();
    let model = crate::semantic::analyze_compilation_with_options(&loaded, crate::semantic::SemanticOptions::modern()).unwrap();
    crate::nir::optimize_program(&crate::nir::lower_program(&crate::semantic::ir::lower_compilation(&loaded, &model))).unwrap()
}

fn program(source: &str, runtime: crate::runtime::Runtime) -> MirProgram {
    crate::mir6502::materialize_program_with_origin_and_runtime(crate::mir6502::lower_program(&checked_nir(source)).unwrap(),
        &crate::mir6502::Mir6502Config { enable_small_leaf_inlining: false, ..crate::mir6502::Mir6502Config::optimized() }, 0x3000, runtime).unwrap()
}

#[test]
fn memory_reviews_separate_result_widths_and_guard_store_footprints() {
    use super::library::{library_invocation_at, reviewed_memory_routine};
    use crate::callee6502::sys_memory::MemoryRoutine::{Peek, PeekC, Poke, PokeC};
    use crate::mir6502::analysis::sites::MirSite;
    use crate::runtime::Runtime;
    let source = "CARD value=$622 BYTE lowResult=$620
        PROC Main()
          lowResult=Peek($D202) value=PeekC($FFFF) lowResult=Peek($D4FF)
          Poke($0610,$34) PokeC($0611,$BEEF)
          PokeC($00A0,$6FDF) PokeC($00A1,$7342) PokeC($3005,$1234)
        RETURN";
    let slot = |byte| MirMem::FixedZeroPage(MirFixedZpSlot(byte));
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let p = program(source, runtime);
        let mut reviews = Vec::new();
        for routine in &p.routines {
            for block in &routine.blocks {
                for (op_index, call) in block.ops.iter().enumerate() {
                    let Some(kind) = reviewed_memory_routine(&p, call) else { continue; };
                    let site = MirSite::Op { block: block.id, op_index };
                    reviews.push((kind, library_invocation_at(&p, routine, site, call, Some(0x3000))));
                }
            }
        }
        assert_eq!(reviews.iter().map(|(kind, _)| *kind).collect::<Vec<_>>(),
            [Peek, PeekC, Peek, Poke, PokeC, PokeC, PokeC, PokeC], "{runtime:?}");
        assert_eq!(reviews[0].1.as_ref().unwrap().definitions, [slot(0xa0)]);
        assert_eq!(reviews[1].1.as_ref().unwrap().definitions, [slot(0xa0), slot(0xa1)]);
        assert_eq!(reviews[0].1.as_ref().unwrap().writes, (0xa0..=0xa3).map(slot).collect::<Vec<_>>());
        assert!(reviews[2].1.is_none());
        assert_eq!(reviews[3].1.as_ref().unwrap().writes, [slot(0xa0), slot(0xa1), MirMem::Absolute(0x0610)]);
        assert_eq!(reviews[4].1.as_ref().unwrap().writes, [
            slot(0xa0), slot(0xa1), MirMem::Absolute(0x0611), MirMem::Absolute(0x0612),
        ]);
        assert!(reviews[5..].iter().all(|(_, review)| review.is_none()), "{runtime:?}");
    }
    let reads = Peek.facts::<()>(Some(0xffff), None).reads.unwrap();
    assert_eq!(reads.iter().map(|range| (range.start, range.end)).collect::<Vec<_>>(),
        [(0xa2, 0xa3), (0xffff, 0xffff), (0, 0)]);
    assert_eq!(PokeC.numeric_observations(0xa0, Some(0xdf)), [(0xa0, 1), (0xe0, 1)]);
    assert_eq!(PokeC.numeric_observations(0xa1, Some(0x42)), [(0xa1, 1), (0x42a2, 1)]);
}

#[test]
fn fixed_memory_argument_admits_a_live_home_while_a_numeric_cursor_stays_conservative() {
    let config = crate::mir6502::Mir6502Config::optimized();
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        for (destination, resident) in [("$0610", true), ("destination", false)] {
            let source = format!(
                "BYTE FUNC Across(BYTE keep CARD destination)
                 Poke({destination},7) keep==+1 RETURN(keep)
                 PROC Main() BYTE out=$600 out=Across(13,$0610) RETURN"
            );
            let output = crate::mir6502::generate_output_with_config_and_runtime(
                &checked_nir(&source), 0x3000, &config, runtime,
            ).unwrap();
            let home = output.map.storage_symbols.iter().find(|symbol| symbol.name == "keep"
                && symbol.scope == crate::codegen::CodegenSymbolScope::Routine("Across".into())).unwrap();
            assert_eq!(home.address < 256, resident, "{runtime:?} {destination} {home:?}");
        }
    }
}

#[test]
fn the_second_peek_read_protects_an_observed_zero_page_byte() {
    let config = crate::mir6502::Mir6502Config::optimized();
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        let home = |address| {
            let source = format!(
                "BYTE observed=$620
                 BYTE FUNC Across(BYTE keep)
                 observed=Peek(${address:04X}) keep==+1 RETURN(keep)
                 PROC Main() BYTE out=$600 out=Across(13) RETURN"
            );
            let output = crate::mir6502::generate_output_with_config_and_runtime(
                &checked_nir(&source), 0x3000, &config, runtime,
            ).unwrap();
            output.map.storage_symbols.iter().find(|symbol| symbol.name == "keep"
                && symbol.scope == crate::codegen::CodegenSymbolScope::Routine("Across".into())).unwrap().address
        };
        let preferred = home(0x0610u16);
        assert!(preferred < 256, "{runtime:?} ${preferred:04X}");
        assert_ne!(home(preferred - 1), preferred, "{runtime:?}");
    }
}

#[test]
fn sound_voice_proof_is_rechecked_after_argument_setup() {
    let config = crate::mir6502::Mir6502Config::optimized();
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        for (voice, resident) in [("1", true), ("voice", false), ("4", false)] {
            let source = format!(
                "BYTE FUNC Across(BYTE keep,voice)
                 Sound({voice},10,4,7) keep==+1 RETURN(keep)
                 PROC Main() BYTE out=$600 out=Across(13,2) RETURN"
            );
            let output = crate::mir6502::generate_output_with_config_and_runtime(
                &checked_nir(&source), 0x3000, &config, runtime,
            ).unwrap();
            let home = output.map.storage_symbols.iter().find(|symbol| symbol.name == "keep"
                && symbol.scope == crate::codegen::CodegenSymbolScope::Routine("Across".into())).unwrap();
            assert_eq!(home.address < 256, resident, "{runtime:?} {voice} {home:?}");
        }
    }
}

#[test]
fn reviewed_hardware_calls_define_results_and_check_guarded_operands() {
    use crate::callee6502::{sys_hardware::HardwareRoutine::*, sys_review::SysReview::Hardware};
    use crate::mir6502::analysis::sites::MirSite;
    use super::library::library_invocation_at;
    let source = "BYTE ARRAY observed(6)=$600
    PROC Main()
        observed(0)=Paddle(2) observed(1)=PTrig(1) observed(2)=Stick(3) observed(3)=STrig(2)
        observed(4)=Rand(0) observed(5)=Rand(10) Sound(2,3,4,5) SndRst()
        Sound(4,3,4,5) observed(1)=PTrig(200)
    RETURN";
    let slot = |value| MirMem::FixedZeroPage(MirFixedZpSlot(value));
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        let p = program(source, runtime);
        let mut witnessed = Vec::new();
        for routine in &p.routines {
            let Some(values) = buffers::argument_values(&p, routine) else { continue; };
            for block in &routine.blocks {
                for (op_index, call) in block.ops.iter().enumerate() {
                    let MirOp::Call { abi, .. } = call else { continue; };
                    let Some(Hardware(hardware)) = abi.library.as_ref().and_then(|b| b.sys_review) else { continue; };
                    let site = MirSite::Op { block: block.id, op_index };
                    let reviewed = library_invocation_at(&p, routine, site, call, None).map(|invocation| {
                        let writes = invocation.normal_writes(&p, call, Some((&values, site)));
                        (invocation.definitions, writes)
                    });
                    witnessed.push((hardware, reviewed));
                }
            }
        }
        let result = Some((vec![slot(0xa0)], MirKnownMemoryWrites::Exact(vec![slot(0xa0)])));
        for hardware in [Paddle, PTrig, Stick, STrig] {
            assert!(witnessed.contains(&(hardware, result.clone())), "{runtime:?} {witnessed:?}");
        }
        assert!(witnessed.contains(&(Rand, result)));
        assert!(witnessed.iter().any(|(hw, footprint)| *hw == Rand && matches!(footprint,
            Some((_, MirKnownMemoryWrites::Exact(writes))) if writes.contains(&slot(0x84)))));
        assert!(witnessed.contains(&(Sound, Some((vec![], MirKnownMemoryWrites::Exact(vec![
            slot(0xa2), MirMem::Absolute(0xd204), MirMem::Absolute(0xd205),
        ]))))), "{runtime:?} {witnessed:?}");
        assert!(witnessed.contains(&(Sound, None)) && witnessed.contains(&(PTrig, None)));
        assert!(witnessed.iter().any(|(hw, footprint)| *hw == SndRst && matches!(footprint,
            Some((_, MirKnownMemoryWrites::Exact(writes))) if writes.contains(&MirMem::Absolute(0xd20f)))));
    }
}

#[test]
fn cio_character_reviews_match_both_providers_and_keep_dispatch_open() {
    use crate::callee6502::sys_cio::{CioCommand::Fixed, CioRoutine};
    let source = "BYTE character=$600 PROC Main()
                  PutD(0,'X) character=GetD(0) RETURN";
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        let p = program(source, runtime);
        let calls = p.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops)
            .filter_map(|call| Some((call, reviewed_cio_transfer(&p, call)?))).collect::<Vec<_>>();
        assert_eq!(calls.len(), 2, "{runtime:?}");
        assert_eq!(calls[0].1, CioRoutine::PutD.transfer(runtime));
        assert_eq!(calls[1].1, CioRoutine::GetD.transfer(runtime));
        assert_eq!(calls[0].1.command, Fixed(0x0b));
        assert_eq!(calls[1].1.successful_result, Some(0xa0));
        assert!(calls.iter().all(|(call, _)| library_invocation(&p, call).is_none()));
        if runtime == crate::runtime::Runtime::Standalone {
            let mut changed = p.clone();
            let ccio = changed.routines.iter_mut().find(|r| r.name.ends_with("::CCIO")).unwrap();
            ccio.blocks.clear();
            assert!(calls.iter().all(|(call, _)| reviewed_cio_transfer(&changed, call).is_none()));
        }
    }
}

#[test]
fn cio_character_output_reviews_capture_the_channel_and_source_byte() {
    use crate::callee6502::sys_cio::{CioBuffer, CioCharacter, CioCommand, CioDevice};
    use crate::runtime::Runtime;

    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for (source, device, character) in [
            ("Put('Q)", CioDevice::Current, CioCharacter::Argument(0)),
            ("PutE()", CioDevice::Current, CioCharacter::Fixed(0x9b)),
            ("PutDE(2)", CioDevice::Argument(0), CioCharacter::Fixed(0x9b)),
            ("PutD(2,'Q)", CioDevice::Argument(0), CioCharacter::Argument(1)),
        ] {
            let p = program(&format!("PROC Main() {source} RETURN"), runtime);
            let (call, transfer) = p.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops)
                .find_map(|call| Some((call, reviewed_cio_transfer(&p, call)?)))
                .unwrap_or_else(|| panic!("missing review for {source} on {runtime:?}"));
            assert_eq!(transfer.device, device);
            assert_eq!(transfer.command, CioCommand::Fixed(0x0b));
            assert_eq!(transfer.buffer, CioBuffer::Character(character));
            assert!(library_invocation(&p, call).is_none());
            if runtime == Runtime::Standalone {
                let mut changed = p.clone();
                changed.routines.iter_mut().find(|r| r.name.ends_with("::PutD1")).unwrap().blocks.clear();
                assert!(reviewed_cio_transfer(&changed, call).is_none());
            }
        }
    }
}

#[test]
fn cio_string_output_reviews_distinguish_byte_and_record_dispatch() {
    use crate::callee6502::sys_cio::{CioBuffer, CioCommand, CioDevice};
    use crate::runtime::Runtime;

    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for (source, device, argument, command, end_on_zero, helper) in [
            ("Print($0610)", CioDevice::Current, 0, 0x0b, None, "::Output"),
            ("PrintD(2,$0610)", CioDevice::Argument(0), 1, 0x0b, None, "::Output"),
            ("PrintE($0610)", CioDevice::Current, 0, 0x09, Some(0x9b), "::Prt"),
            ("PrintDE(2,$0610)", CioDevice::Argument(0), 1, 0x09, Some(0x9b), "::Prt"),
        ] {
            let p = program(&format!("PROC Main() {source} RETURN"), runtime);
            let (call, transfer) = p.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops)
                .find_map(|call| Some((call, reviewed_cio_transfer(&p, call)?)))
                .unwrap_or_else(|| panic!("missing review for {source} on {runtime:?}"));
            assert_eq!(transfer.device, device);
            assert_eq!(transfer.command, CioCommand::Fixed(command));
            assert_eq!(transfer.buffer, CioBuffer::StringOutput { argument, end_on_zero });
            assert!(library_invocation(&p, call).is_none());
            if runtime == Runtime::Standalone {
                let mut changed = p.clone();
                changed.routines.iter_mut().find(|r| r.name.ends_with(helper)).unwrap().blocks.clear();
                assert!(reviewed_cio_transfer(&changed, call).is_none());
            }
        }
    }
}

#[test]
fn cio_open_and_close_bind_the_selected_filename_and_command_paths() {
    use crate::callee6502::sys_cio::{CioBuffer, CioCommand::Fixed, FilenameBuffer};
    use crate::runtime::Runtime;
    let source = "PROC Main() Open(1,\"D:TEST\",4,0) Close(1) RETURN";
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let p = program(source, runtime);
        let calls = p.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops)
            .filter_map(|call| Some((call, reviewed_cio_transfer(&p, call)?))).collect::<Vec<_>>();
        assert_eq!(calls.len(), 2, "{runtime:?}");
        assert_eq!(calls[0].1.command, Fixed(3));
        assert_eq!(calls[1].1.command, Fixed(12));
        let destination = match runtime {
            Runtime::ActionCart => FilenameBuffer::PointerCell(0x009b),
            Runtime::Standalone => FilenameBuffer::Fixed(0x0500),
        };
        assert_eq!(calls[0].1.buffer, CioBuffer::FilenameCopy { argument: 1, destination });
        assert!(calls.iter().all(|(call, _)| library_invocation(&p, call).is_none()));
        if runtime == Runtime::Standalone {
            let mut changed = p.clone();
            let xiostr = changed.routines.iter_mut().find(|r| r.name.ends_with("::XIOstr")).unwrap();
            xiostr.blocks.clear();
            assert!(calls.iter().all(|(call, _)| reviewed_cio_transfer(&changed, call).is_none()));
        }
    }
}

#[test]
fn cio_record_input_reviews_bind_all_entry_wrappers_and_keep_dispatch_open() {
    use crate::callee6502::sys_cio::{CioBuffer, CioCommand, CioDevice, InputMaximum};
    use crate::runtime::Runtime;

    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for (source, device, argument, maximum) in [
            ("InputD(1,$0610)", CioDevice::Argument(0), 1, InputMaximum::DestinationHeader),
            ("InputMD(1,$0610,7)", CioDevice::Argument(0), 1, InputMaximum::Argument(2)),
            ("InputSD(1,$0610)", CioDevice::Argument(0), 1, InputMaximum::Fixed(255)),
            ("InputS($0610)", CioDevice::Current, 0, InputMaximum::Fixed(255)),
        ] {
            let p = program(&format!("PROC Main() {source} RETURN"), runtime);
            let (call, transfer) = p.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops)
                .find_map(|call| Some((call, reviewed_cio_transfer(&p, call)?)))
                .unwrap_or_else(|| panic!("missing review for {source} on {runtime:?}"));
            assert_eq!(transfer.device, device);
            assert_eq!(transfer.command, CioCommand::Fixed(5));
            assert_eq!(transfer.buffer, CioBuffer::RecordInput { argument, maximum });
            assert!(transfer.successful_result.is_none());
            assert!(library_invocation(&p, call).is_none());
            if runtime == Runtime::Standalone {
                let mut changed = p.clone();
                changed.routines.iter_mut().find(|r| r.name.ends_with("::InS")).unwrap().blocks.clear();
                assert!(reviewed_cio_transfer(&changed, call).is_none());
            }
        }
    }
}

#[test]
fn cio_seek_reviews_distinguish_note_writeback_from_point_setup() {
    use crate::callee6502::sys_cio::{CioBuffer, CioCommand, CioDevice, CioPosition};
    use crate::runtime::Runtime;

    let source = "CARD sector=$610 BYTE offset=$612
                  PROC Main() Note(1,@sector,@offset) Point(1,sector,offset) RETURN";
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let p = program(source, runtime);
        let calls = p.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops)
            .filter_map(|call| Some((call, reviewed_cio_transfer(&p, call)?))).collect::<Vec<_>>();
        let [(note, read), (point, write)] = calls.as_slice() else { panic!("missing seek reviews on {runtime:?}"); };
        assert_eq!(read.command, CioCommand::Fixed(0x26));
        assert_eq!(write.command, CioCommand::Fixed(0x25));
        assert_eq!(read.position, Some(CioPosition::ReadBack { sector_pointer: 1, offset_pointer: 2 }));
        assert_eq!(write.position, Some(CioPosition::Set { sector_argument: 1, offset_argument: 2 }));
        for &(call, transfer) in &calls {
            assert_eq!(transfer.device, CioDevice::Argument(0));
            assert_eq!(transfer.buffer, CioBuffer::NoTransfer);
            assert!(transfer.successful_result.is_none());
            assert!(library_invocation(&p, call).is_none());
        }
        if runtime == Runtime::Standalone {
            let mut changed = p.clone();
            changed.routines.iter_mut().find(|r| r.name.ends_with("::ChkErr")).unwrap().blocks.clear();
            assert!(reviewed_cio_transfer(&changed, note).is_none());
            assert!(reviewed_cio_transfer(&changed, point).is_none());
        }
    }
}

#[test]
fn xio_reviews_the_public_command_lane_and_counted_string_without_a_device_proof() {
    use crate::callee6502::sys_cio::{CioBuffer, CioCommand};
    use crate::runtime::Runtime;
    let source = "MODULE CLIENT USE SYS
                  PROC Main() SYS.XIO(4,9,123,0,42,\"D:\") RETURN ENDMODULE";
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let p = program(source, runtime);
        let (call, transfer) = p.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops)
            .find_map(|call| Some((call, reviewed_cio_transfer(&p, call)?))).unwrap();
        assert_eq!(transfer.command, CioCommand::Argument(2));
        assert_eq!(transfer.buffer, CioBuffer::CountedString { argument: 5 });
        assert_eq!(transfer.conditional_aux, Some((3, 4)));
        assert!(library_invocation(&p, call).is_none());
        let mut invalid = call.clone();
        let MirOp::Call { args, .. } = &mut invalid else { unreachable!(); };
        args.pop();
        assert!(reviewed_cio_transfer(&p, &invalid).is_none());
    }
    let shadow = program("PROC XIO() RETURN PROC Main() XIO() RETURN", Runtime::ActionCart);
    assert!(shadow.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops)
        .all(|call| reviewed_cio_transfer(&shadow, call).is_none()));
}

#[test]
fn plot_and_locate_reviews_keep_the_cio_boundary_open() {
    let source = "BYTE colorResult=$0600 PROC Main()
                  Plot(10,20) colorResult=Locate(10,20) RETURN";
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        let p = program(source, runtime);
        let calls = p.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops)
            .filter(|call| graphics_cio_boundary(&p, call).is_some()).collect::<Vec<_>>();
        assert_eq!(calls.len(), 2, "{runtime:?}");
        assert_eq!(graphics_cio_boundary(&p, calls[0]), Some("PutD on S: (reads COLOR at $02FD)"));
        assert_eq!(graphics_cio_boundary(&p, calls[1]), Some("GetD on S: (successful pixel result at $A0)"));
        assert!(calls.iter().all(|call| library_invocation(&p, call).is_none()));
        if runtime == crate::runtime::Runtime::Standalone {
            let mut changed = p.clone();
            let position = changed.routines.iter_mut().find(|r| r.name.ends_with("::Pos1")).unwrap();
            position.blocks.clear();
            assert!(calls.iter().all(|call| graphics_cio_boundary(&changed, call).is_none()));
        }
    }
}

#[test]
fn draw_and_fill_share_grio_without_sharing_command_identity() {
    let source = "PROC Main() DrawTo(12,22) Fill(14,24) RETURN";
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        let p = program(source, runtime);
        let calls = p.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops)
            .filter(|call| graphics_cio_boundary(&p, call).is_some()).collect::<Vec<_>>();
        assert_eq!(calls.len(), 2, "{runtime:?}");
        assert_eq!(graphics_cio_boundary(&p, calls[0]), Some("XIO $11 on S: (reads COLOR and writes $02FB)"));
        assert_eq!(graphics_cio_boundary(&p, calls[1]), Some("XIO $12 on S: (reads COLOR and writes $02FB)"));
        assert!(calls.iter().all(|call| library_invocation(&p, call).is_none()));
        if runtime == crate::runtime::Runtime::Standalone {
            let mut changed = p.clone();
            let grio = changed.routines.iter_mut().find(|r| r.name.ends_with("::GrIO")).unwrap();
            grio.blocks.clear();
            assert!(calls.iter().all(|call| graphics_cio_boundary(&changed, call).is_none()));
        }
    }
}

#[test]
fn graphics_reports_the_mode_change_boundary_without_preservation() {
    let source = "MODULE CLIENT USE SYS PROC Main() SYS.Graphics($18) RETURN ENDMODULE";
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        let p = program(source, runtime);
        let calls = p.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops)
            .filter(|call| graphics_cio_boundary(&p, call).is_some()).collect::<Vec<_>>();
        assert_eq!(calls.len(), 1, "{runtime:?}");
        let call = calls[0];
        assert_eq!(graphics_cio_boundary(&p, call),
            Some("Close/Open on E: and S: (screen mode in AUX2, derived access in AUX1)"));
        assert!(library_invocation(&p, call).is_none());
        if runtime == crate::runtime::Runtime::Standalone {
            let mut changed = p.clone();
            let open = changed.routines.iter_mut().find(|r| r.name.ends_with("::Open")).unwrap();
            open.blocks.clear();
            assert!(graphics_cio_boundary(&changed, call).is_none());
        }
    }

    let shadow = program("PROC Graphics(BYTE mode) RETURN PROC Main() Graphics(8) RETURN",
        crate::runtime::Runtime::ActionCart);
    assert!(shadow.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops)
        .all(|op| graphics_cio_boundary(&shadow, op).is_none()));
}

#[test]
fn setcolor_can_preserve_a_caller_home_when_the_register_is_known() {
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        for register in ["2", "register"] {
            let source = format!(
                "BYTE FUNC Across(BYTE keep,register)
                 SetColor({register},10,12)
                 keep==+1
                 RETURN(keep)
                 PROC Main() BYTE out=$600 out=Across(13,2) RETURN"
            );
            let output = crate::mir6502::generate_output_with_config_and_runtime(
                &checked_nir(&source), 0x3000, &crate::mir6502::Mir6502Config::optimized(), runtime,
            ).unwrap();
            let home = output.map.storage_symbols.iter().find(|symbol| symbol.name == "keep"
                && symbol.scope == crate::codegen::CodegenSymbolScope::Routine("Across".into())).unwrap();
            assert_eq!(home.address < 256, register == "2", "{runtime:?} {register} {home:?}");
        }
    }
}

#[test]
fn graphics_leaf_reviews_cover_fallthrough_and_guard_color_writes() {
    let source = "MODULE CLIENT USE SYS
PROC Color(BYTE register) SYS.SetColor(register,10,12) RETURN
PROC Main() SYS.Position($1234,$56) SYS.SetColor(2,10,12) SYS.SetColor(133,10,12) RETURN ENDMODULE";
    use crate::callee6502::sys_graphics::GraphicsRoutine;
    use crate::mir6502::analysis::sites::MirSite;
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        let p = program(source, runtime);
        let mut writes = Vec::new();
        let mut linked = None;
        for routine in &p.routines {
            for block in &routine.blocks {
                for (op_index, call) in block.ops.iter().enumerate() {
                    let MirOp::Call { abi, .. } = call else { continue; };
                    if abi.library.is_none() { continue; }
                    let summary = library_invocation(&p, call).unwrap_or_else(||
                        panic!("missing {runtime:?} graphics review: {call:?}"));
                    let values = buffers::argument_values(&p, routine).unwrap();
                    let site = MirSite::Op { block: block.id, op_index };
                    writes.push((match summary.sys_review.unwrap() { crate::callee6502::sys_review::SysReview::Graphics(graphics) => graphics, _ => unreachable!() }, summary.normal_writes(&p, call, Some((&values, site)))));
                    if summary.sys_review == Some(crate::callee6502::sys_review::SysReview::Graphics(GraphicsRoutine::Position)) {
                        linked = Some(call);
                    }
                }
            }
        }
        assert!(writes.contains(&(GraphicsRoutine::Position, MirKnownMemoryWrites::Exact(
            (0x54..=0x56).chain(0x5a..=0x5c).map(|n| MirMem::FixedZeroPage(MirFixedZpSlot(n))).collect()
        ))), "{writes:?}");
        assert!(writes.contains(&(GraphicsRoutine::SetColor, MirKnownMemoryWrites::Exact(vec![
            MirMem::FixedZeroPage(MirFixedZpSlot(0xa0)), MirMem::FixedZeroPage(MirFixedZpSlot(0xa2)),
            MirMem::Absolute(0x02c6), MirMem::Absolute(0xd018),
        ]))), "{writes:?}");
        assert_eq!(writes.iter().filter(|(_, w)| *w == MirKnownMemoryWrites::Unknown).count(), 2);
        if runtime == crate::runtime::Runtime::Standalone {
            let mut changed = p.clone();
            let pos1 = changed.routines.iter_mut().find(|r| r.name.ends_with("::Pos1")).unwrap();
            pos1.blocks.clear();
            assert!(library_invocation(&changed, linked.unwrap()).is_none());
        }
    }
}

#[test]
fn library_reader_contract_keeps_interface_identity_and_selected_body_evidence() {
    let source = "MODULE CLIENT USE SYS
BYTE ARRAY left=\"A\",right=\"B\" INT comparison BYTE byteValue CARD cardValue
INT FUNC Compare() RETURN(SYS.SCompare(left,right))
PROC Main() comparison=Compare() byteValue=SYS.Peek($600) cardValue=SYS.PeekC($600) RETURN ENDMODULE";
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        let p = program(source, runtime);
        let r = p.routines.iter().find(|r| r.name.starts_with("M_CLIENT_COMPARE_")).unwrap();
        let call = r.blocks.iter().flat_map(|b| &b.ops).find(|op| matches!(op, MirOp::Call { abi, .. } if abi.library.is_some())).unwrap();
        let contract = library_invocation(&p, call).unwrap_or_else(|| panic!("missing library contract: {call:?}"));
        assert_eq!(contract.inputs.len(), 4);
        assert_eq!(contract.definitions, vec![MirMem::FixedZeroPage(MirFixedZpSlot(0xa0)), MirMem::FixedZeroPage(MirFixedZpSlot(0xa1))]);
        assert_eq!(contract.writes.len(), 6);
        let entry = MirAcyclicEntryAnalysis::new(&p).analyze(r.id).unwrap();
        assert!(entry.inputs.fixed.is_empty());
        assert!(!entry.inputs.flags.any());
        assert_eq!(entry.defined_results, BTreeSet::from([0xa0,0xa1]));
        let full = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
        assert_eq!(full.get(r.id).unwrap().reads(), &MirKnownMemoryReads::Unknown);
        assert!(matches!(full.get(r.id).unwrap().writes(), MirKnownMemoryWrites::Exact(_)));
        let peeks: Vec<_> = p.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops).filter_map(|op| match op {
            MirOp::Call { abi, .. } => abi.library.as_ref().filter(|b| b.interface.name.to_ascii_uppercase().ends_with(".PEEK") || b.interface.name.to_ascii_uppercase().ends_with(".PEEKC")), _ => None,
        }).collect();
        assert_eq!(peeks.len(), 2);
        assert_ne!(peeks[0].interface.id, peeks[1].interface.id);
        assert_ne!(peeks[0].interface.direct_result_abi, peeks[1].interface.direct_result_abi);
        if runtime == crate::runtime::Runtime::ActionCart {
            assert_eq!(peeks[0].implementation.as_ref().unwrap().target, peeks[1].implementation.as_ref().unwrap().target);
        }
        let mut invalid = call.clone();
        let MirOp::Call { args, .. } = &mut invalid else { unreachable!() };
        args.pop();
        assert!(library_invocation(&p, &invalid).is_none());
        let mut invalid = call.clone();
        let MirOp::Call { abi, .. } = &mut invalid else { unreachable!() };
        abi.library = None;
        assert!(library_invocation(&p, &invalid).is_none());
        let MirOp::Call { abi, .. } = call else { unreachable!() };
        if let MirRuntimeHelperTarget::Routine(id) = abi.library.as_ref().unwrap().implementation.as_ref().unwrap().target {
            let mut changed = p.clone();
            changed.routines.iter_mut().find(|r| r.id == id).unwrap().blocks[0].ops.clear();
            assert!(library_invocation(&changed, call).is_none());
            assert!(!entry.is_current(&changed, MirCalleeEntryStage::ActualEntry));
        }
    }
    let shadow = program("INT FUNC SCompare(INT a,b) RETURN(a+b) PROC Main() SCompare(1,2) RETURN", crate::runtime::Runtime::ActionCart);
    assert!(shadow.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops).all(|op| !matches!(op, MirOp::Call { abi, .. } if abi.library.is_some())));
}

#[test]
fn buffer_writes_keep_numeric_ranges_private_permissions_and_unknown_full_effects() {
    use crate::mir6502::analysis::{sites::MirSite, storage_ownership::ObjectAddressControl};
    let source = "MODULE CLIENT USE SYS
BYTE ARRAY source(256)=$900,destination(256)=$B00 BYTE POINTER ptr=$700
PROC Copies() SYS.SCopy(destination,source) SYS.SCopy($E0,source) RETURN
PROC Dynamic() SYS.SCopy(ptr,source) RETURN
PROC Forward(STRING dst) SYS.SCopy(dst,source) RETURN
BYTE FUNC Exposed(BYTE keep) SYS.SCopy(@keep,source) RETURN(keep)
PROC Main() RETURN ENDMODULE";
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        let p = program(source, runtime);
        let r = p.routines.iter().find(|r| r.name.starts_with("M_CLIENT_COPIES_")).unwrap();
        let values = buffers::argument_values(&p, r).unwrap();
        let ranges: Vec<_> = r.blocks.iter().flat_map(|b| b.ops.iter().enumerate().filter_map(|(op_index, op)| {
            let contract = library_invocation(&p, op)?;
            assert_eq!(contract.buffers.len(), 2);
            assert!(contract.definitions.is_empty());
            Some(contract.buffers.iter().find(|b| b.write).unwrap().project(&p, op,
                Some((&values, MirSite::Op { block: b.id, op_index }))))
        })).collect();
        assert_eq!(ranges, [(Some(0xb00),256),(Some(0xe0),256)]);
        let full = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
        assert_eq!(full.get(r.id).unwrap().writes(), &MirKnownMemoryWrites::Unknown);
        assert_eq!(full.get(r.id).unwrap().reads(), &MirKnownMemoryReads::Unknown);
        let normal = MirKnownCalleeSummaries::analyze_for_residence(&p);
        let writes = normal.get(r.id).unwrap().writes();
        assert!(!writes.preserves_private_fixed_byte(MirFixedZpSlot(0xe4), normal.fixed_storage()));
        let forward = p.routines.iter().find(|r| r.name.starts_with("M_CLIENT_FORWARD_")).unwrap();
        assert_eq!(normal.get(forward.id).unwrap().writes(), &MirKnownMemoryWrites::Unknown);
        let dynamic = p.routines.iter().find(|r| r.name.starts_with("M_CLIENT_DYNAMIC_")).unwrap();
        let writes = normal.get(dynamic.id).unwrap().writes();
        assert!(matches!(writes, MirKnownMemoryWrites::Pointee { numeric, .. } if numeric.is_empty()));
        assert!(!writes.preserves_fixed_byte(MirFixedZpSlot(0xe4), normal.fixed_storage()));
        assert!(writes.preserves_private_fixed_byte(MirFixedZpSlot(0xe4), normal.fixed_storage()));
        assert!(!writes.preserves_private_fixed_byte(MirFixedZpSlot(0xa1), normal.fixed_storage()));
        let exposed = p.routines.iter().find(|r| r.name.starts_with("M_CLIENT_EXPOSED_")).unwrap();
        let MirStorageBase::Param(id) = exposed.frame.params[0].base else { panic!() };
        assert!(!ObjectAddressControl::new(&p).compiler_controls_address(exposed.id, &MirMem::Param { id, offset: 0 }));
    }
}

#[test]
fn memory_library_contracts_cover_counts_later_lanes_and_fallthrough() {
    use crate::mir6502::analysis::sites::MirSite;
    let source = "MODULE CLIENT USE SYS
PROC Calls()
SYS.Zero($FFFE,0) SYS.Zero($FFFE,3)
SYS.SetBlock($700,0,7) SYS.SetBlock($700,2,7)
SYS.MoveBlock($800,$900,257)
RETURN PROC Main() RETURN ENDMODULE";
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        let p = program(source, runtime);
        let r = p.routines.iter().find(|r| r.name.starts_with("M_CLIENT_CALLS_")).unwrap();
        let values = buffers::argument_values(&p, r).unwrap();
        let fixed = crate::mir6502::analysis::known_callees::MirFixedStorage::new(&p);
        let mut writes = Vec::new();
        for b in &r.blocks { for (op_index, call) in b.ops.iter().enumerate() {
            let Some(contract) = library_invocation(&p, call) else { continue; };
            assert!(contract.definitions.is_empty());
            let MirOp::Call { abi, .. } = call else { panic!() };
            let binding = abi.library.as_ref().unwrap();
            if binding.kind == Some(MirLibraryRoutine::MoveBlock) {
                assert_eq!(contract.inputs.len(), 6);
                let mut bad = call.clone();
                let MirOp::Call { args, .. } = &mut bad else { unreachable!() };
                args.pop();
                assert!(library_invocation(&p, &bad).is_none());
            }
            if binding.kind == Some(MirLibraryRoutine::Zero) && runtime == crate::runtime::Runtime::Standalone {
                let implementation = binding.implementation.as_ref().unwrap();
                assert!(!implementation.adjacent_entries.is_empty());
                let (_, dependency) = implementation.adjacent_entries[0];
                let mut changed = p.clone();
                changed.routines.iter_mut().find(|r| r.id == dependency).unwrap().blocks[0].ops.clear();
                assert!(library_invocation(&changed, call).is_none());
            }
            let footprint = contract.normal_writes(&p, call, Some((&values, MirSite::Op { block: b.id, op_index })));
            assert!(!footprint.preserves_private_fixed_byte(MirFixedZpSlot(0xa1), &fixed));
            writes.push(footprint);
        }}
        assert_eq!(writes.len(), 5);
        assert!(matches!(writes[0], MirKnownMemoryWrites::Exact(_)));
        assert!(matches!(&writes[1], MirKnownMemoryWrites::Pointee { numeric, .. } if numeric == &[(0xfffe,3)]));
        assert!(!writes[1].preserves_private_fixed_byte(MirFixedZpSlot(0), &fixed));
        assert!(writes[1].preserves_private_fixed_byte(MirFixedZpSlot(1), &fixed));
        assert!(matches!(writes[2], MirKnownMemoryWrites::Exact(_)));
        assert!(matches!(&writes[3], MirKnownMemoryWrites::Pointee { numeric, .. } if numeric == &[(0x700,2)]));
        assert!(matches!(&writes[4], MirKnownMemoryWrites::Pointee { numeric, .. } if numeric == &[(0x800,257)]), "{:?}", writes[4]);
        for r in &p.routines {
            if r.name.contains("SetBlock") || r.name.contains("MoveBlock") {
                assert!(r.blocks.iter().flat_map(|b| &b.ops).all(|o| !matches!(o, MirOp::SArgsCapture { .. })));
            }
        }
    }
}

#[test]
fn prelude_identity_and_numeric_scratch_collision_remain_explicit() {
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        let p = program("BYTE ARRAY text=\"abc\" PROC Main() SCopy($A0,text) RETURN", runtime);
        let r = p.routines.iter().find(|r| r.name == "Main").unwrap();
        let values = buffers::argument_values(&p, r).unwrap();
        let (block, op_index, call) = r.blocks.iter().flat_map(|b| b.ops.iter().enumerate().map(move |(i,o)|(b.id,i,o)))
            .find(|(_,_,o)| library_invocation(&p,o).is_some()).unwrap();
        let contract = library_invocation(&p,call).unwrap();
        assert_eq!(contract.normal_writes(&p,call,Some((&values,crate::mir6502::analysis::sites::MirSite::Op {block,op_index}))), MirKnownMemoryWrites::Unknown);
    }
}
