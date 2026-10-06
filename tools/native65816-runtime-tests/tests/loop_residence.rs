mod support;
use actionc::mir65816::emit::proof;
use actionc_vm::native65816::Access;
use support::*;

const SCAN: &str = "TYPE Cell=[BYTE flag CARD value Cell POINTER next]\n\
BYTE count=$7000,depth=$7001 CARD result\n\
CARD FUNC Scan(Cell POINTER root,other BYTE count,depth)\n\
 Cell POINTER p CARD sum BYTE i,j\n\
 p=root.next sum=0 i=0\n\
 WHILE i<count DO j=0\n\
  WHILE j<depth DO sum==+p.value other.value=p.value other.value=p.value other.value=p.value p.flag=j p=p.next j==+1 OD\n\
  i==+1 OD RETURN(sum)\n\
PROC Main() result=Scan(Cell POINTER($8100),Cell POINTER($8300),count,depth) RETURN\n";

#[test]
fn nested_record_traversal_preserves_zero_trip_changed_bases_and_exact_accesses() {
    for optimize in [false, true] {
        let compiled = prepare(SCAN, optimize).compile(&layout()).unwrap();
        assert_eq!(
            compiled.image.to_json().unwrap(),
            prepare(&SCAN.replace('\n', "\r\n"), optimize)
                .compile(&layout())
                .unwrap()
                .image
                .to_json()
                .unwrap()
        );
        if optimize {
            assert!(compiled.machine.routines.iter().any(|r| {
                proof::placement_summary(&r.code)
                    .unwrap()
                    .is_some_and(|s| s.loop_homes > 0)
            }));
        }
        for nodes in [[0x8500u32, 0x8700], [0x21ffff, 0x32fffe]] {
            for (count, depth) in [(0, 0), (0, 3), (1, 0), (1, 1), (1, 3), (3, 2)] {
                for mask in [0, 4] {
                    let image = &compiled.image;
                    let mut h = Harness::new(image, &caller(image.entry), mask);
                    for at in [0x8100, 0x8300, nodes[0], nodes[1]] {
                        h.bus.map(at - 1, &[0xa5; 12], true);
                        h.bus.watched.extend(at..at + 10);
                    }
                    h.bus.ram[0x7000] = count;
                    h.bus.ram[0x7001] = depth;
                    h.bus.ram[0x8104..0x8107].copy_from_slice(&nodes[0].to_le_bytes()[..3]);
                    let words = [65535u16, 0x8000];
                    for (index, at) in nodes.into_iter().enumerate() {
                        h.bus.ram[at as usize + 2..at as usize + 4]
                            .copy_from_slice(&words[index].to_le_bytes());
                        h.bus.ram[at as usize + 4..at as usize + 7]
                            .copy_from_slice(&nodes[1 - index].to_le_bytes()[..3]);
                    }
                    let mut expected: Vec<_> =
                        (0x8104..0x8107).map(|a| (a, Access::Read)).collect();
                    let mut sum = 0u16;
                    let mut index = 0;
                    for _ in 0..count {
                        for j in 0..depth {
                            let at = nodes[index];
                            sum = sum.wrapping_add(words[index]);
                            expected.extend((at + 2..at + 4).map(|a| (a, Access::Read)));
                            // Repeated source reads stay separate even though
                            // the captured pointer has a resident home.
                            for _ in 0..3 {
                                expected.extend((at + 2..at + 4).map(|a| (a, Access::Read)));
                                expected.extend(
                                    words[index].to_le_bytes().into_iter().enumerate().map(
                                        |(byte, value)| {
                                            (0x8302 + byte as u32, Access::Write(value))
                                        },
                                    ),
                                );
                            }
                            expected.push((at, Access::Write(j)));
                            expected.extend((at + 4..at + 7).map(|a| (a, Access::Read)));
                            index = 1 - index;
                        }
                    }
                    h.run();
                    h.guards(mask);
                    assert_eq!(h.global(image, "result", 2), u32::from(sum));
                    assert_eq!(
                        h.bus
                            .trace
                            .iter()
                            .map(|(_, at, access)| (*at, *access))
                            .collect::<Vec<_>>(),
                        expected
                    );
                    for at in [0x8100, 0x8300, nodes[0], nodes[1]] {
                        for offset in [-1i32, 1, 7, 10] {
                            assert_eq!(
                                h.bus.ram[(i64::from(at) + i64::from(offset)) as usize],
                                0xa5
                            );
                        }
                    }
                }
            }
        }
    }
}

// The ordinary frontend and ABI supply callable facts. Only the caller's
// captured-value graph is authored, independently of private-local promotion.
#[test]
fn split_captures_survive_direct_indirect_recursive_helper_and_assembly_calls() {
    use actionc::mir65816::{
        Mir65816AddressBase as Base, Mir65816CallTarget as Target, Mir65816Op as Op,
        Mir65816Value as V, abi, image::AssemblyImport,
    };
    use actionc::nir::{ByteOffset, ByteSize, TempId, runtime_symbol_id};
    let pointer_size = ByteSize::new(3);
    let word_size = ByteSize::new(2);
    for kind in ["direct", "indirect", "recursive", "helper", "assembly"] {
        let invocation = match kind {
            "direct" => "Touch(other)",
            "indirect" => "cb(other)",
            "recursive" => "Recurse(other,2)",
            "helper" => "amount*stride",
            _ => "Clobber(other)",
        };
        let source = format!(
            "MODULE TEST TYPE Cell=[BYTE flag CARD value Cell POINTER next]\n\
PUBLIC EXTERNAL CARD FUNC Clobber(Cell POINTER p)\n\
CARD answer CARD FUNC POINTER cb(Cell POINTER p)\n\
CARD FUNC Touch(Cell POINTER p) RETURN(Clobber(p))\n\
CARD FUNC Recurse(Cell POINTER p BYTE depth) IF depth THEN Recurse(p,depth-1) FI RETURN(Touch(p))\n\
PROC Work(Cell POINTER root,other CARD amount,stride) LET saved=root.next LET observed=saved.value answer={invocation} RETURN\n\
PROC Main() cb=@Touch Work(Cell POINTER($8100),Cell POINTER($8300),$1234,3) RETURN ENDMODULE"
        );
        let mut prepared = prepare(&source, false);
        let r = prepared
            .mir
            .routines
            .iter_mut()
            .find(|r| r.name == "Work" || r.name.starts_with("M_TEST_WORK_"))
            .unwrap();
        let root = r.frame.parameters[0].param;
        let other = r.frame.parameters[1].param;
        let mut call = r
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .find(|op| matches!(op, Op::Call { .. }))
            .unwrap()
            .clone();
        let pointer = r
            .temps
            .iter()
            .find(|(_, ty)| ty.pointer)
            .map(|(_, ty)| ty.clone())
            .unwrap();
        let word = r
            .temps
            .iter()
            .find(|(_, ty)| ty.width == Some(word_size))
            .unwrap()
            .1
            .clone();
        let mut address = r
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .find_map(|op| {
                if let Op::Load { address, .. } = op {
                    Some(address.clone())
                } else {
                    None
                }
            })
            .unwrap();
        address.index = None;
        address.mode = actionc::mir65816::Mir65816AddressMode::LongIndirect;
        let place = |base, offset| {
            let mut a = address.clone();
            a.base = Base::Indirect(base);
            a.displacement = ByteOffset::new(offset);
            a
        };
        let p = V::Temp(TempId(0), pointer_size);
        let n = V::Temp(TempId(1), word_size);
        let mut target_load = None;
        if let Op::Call {
            args,
            target,
            result,
            ..
        } = &mut call
        {
            *result = Some((TempId(2), word_size));
            if kind == "helper" {
                args[0] = V::U16(0x1234);
                args[1] = V::U16(3);
            } else {
                args[0] = V::Param(other);
            }
            if let Target::Indirect(value, _) = target {
                // Lowering stages the callee in an automatic object before
                // the call. Rebuild its original global capture, not a read
                // of that discarded staging object's uninitialized bytes.
                target_load = r
                    .blocks
                    .iter()
                    .flat_map(|b| &b.ops)
                    .find(|op| {
                        matches!(op,
                    Op::Load { width, address, .. } if *width == pointer_size
                        && matches!(address.base, Base::External(_) | Base::Static(_)))
                    })
                    .cloned();
                if let Some(Op::Load { dest, .. }) = &mut target_load {
                    *dest = TempId(3);
                }
                *value = V::Temp(TempId(3), pointer_size);
            }
        }
        r.blocks.truncate(1);
        let mut ops = vec![
            Op::Load {
                dest: TempId(0),
                width: pointer_size,
                address: place(V::Param(root), 4),
                volatile: false,
            },
            Op::Load {
                dest: TempId(1),
                width: word_size,
                address: place(p.clone(), 2),
                volatile: false,
            },
        ];
        for _ in 0..3 {
            for base in [V::Param(other), p.clone()] {
                ops.push(Op::Store {
                    address: place(base, 0),
                    value: V::U8(9),
                    width: ByteSize::ONE,
                    volatile: false,
                });
            }
        }
        if let Some(op) = target_load {
            ops.push(op);
        }
        ops.push(call);
        for _ in 0..3 {
            for base in [p.clone(), V::Param(other)] {
                ops.push(Op::Store {
                    address: place(base, 0),
                    value: V::U8(11),
                    width: ByteSize::ONE,
                    volatile: false,
                });
            }
        }
        ops.extend([
            Op::Store {
                address: place(p.clone(), 2),
                value: n,
                width: word_size,
                volatile: false,
            },
            Op::Load {
                dest: TempId(4),
                width: word_size,
                address: place(V::Param(other), 2),
                volatile: false,
            },
            Op::Store {
                address: place(p, 2),
                value: V::Temp(TempId(4), word_size),
                width: word_size,
                volatile: false,
            },
            Op::Store {
                address: place(V::Param(other), 2),
                value: V::Temp(TempId(2), word_size),
                width: word_size,
                volatile: false,
            },
        ]);
        r.blocks[0].ops = ops;
        r.temps = vec![
            (TempId(0), pointer.clone()),
            (TempId(1), word.clone()),
            (TempId(2), word.clone()),
            (TempId(4), word),
        ];
        if kind == "indirect" {
            r.temps.push((TempId(3), pointer));
        }
        // Repeat the complete call-containing body. The captured pointer and
        // word survive both the loop backedge and every call; each segment is
        // independently reloaded on every dynamic visit.
        use actionc::mir65816::{
            Mir65816Block as Block, Mir65816Edge as Edge, Mir65816Terminator as Term,
        };
        use actionc::nir::{BlockId, NirBinaryOp, NirCompareOp};
        let mut body = r.blocks[0].ops.split_off(2);
        let done = r.blocks[0].terminator.clone();
        let ty = r
            .temps
            .iter()
            .find(|(id, _)| *id == TempId(1))
            .unwrap()
            .1
            .clone();
        r.temps.extend((5..7).map(|id| (TempId(id), ty.clone())));
        r.temps.push((
            TempId(7),
            actionc::nir::NirType {
                kind: actionc::nir::NirTypeKind::Bool,
                summary: "bool".into(),
                width: Some(ByteSize::ONE),
                pointer: false,
            },
        ));
        body.extend([
            Op::Binary {
                dest: TempId(6),
                width: word_size,
                signed: false,
                operation: NirBinaryOp::Sub,
                left: V::Temp(TempId(5), word_size),
                right: V::U16(1),
            },
            Op::Compare {
                dest: TempId(7),
                width: word_size,
                signed: false,
                operation: NirCompareOp::Ne,
                left: V::Temp(TempId(6), word_size),
                right: V::U16(0),
            },
        ]);
        r.blocks[0].terminator = Term::Goto(Edge {
            target: BlockId(1),
            args: vec![V::U16(2)],
        });
        r.blocks.extend([
            Block {
                id: BlockId(1),
                params: vec![(TempId(5), word_size)],
                ops: body,
                terminator: Term::Branch {
                    condition: V::Temp(TempId(7), ByteSize::ONE),
                    then_edge: Edge {
                        target: BlockId(1),
                        args: vec![V::Temp(TempId(6), word_size)],
                    },
                    else_edge: Edge {
                        target: BlockId(2),
                        args: vec![],
                    },
                },
            },
            Block {
                id: BlockId(2),
                params: vec![],
                ops: vec![],
                terminator: done,
            },
        ]);
        // Keep the canonical source frame; no call-saved scratch is added.
        actionc::mir65816::verify_program(&prepared.mir).unwrap();
        let symbol = runtime_symbol_id("TEST.Clobber");
        let external = prepared
            .mir
            .routines
            .iter()
            .find(|r| r.entry.external_symbol == Some(symbol))
            .unwrap();
        let mut options = layout();
        let assembly = assemble(
            "rep #$30\nldx #$80\nlda #$5a5a\nclobber: sta $00,x\ninx\ninx\ncpx #$c0\nbcc clobber\nlda 4,s\nsta $80\nlda 5,s\nsta $81\nldy #2\nlda #$beef\nsta [$80],y\nlda #$eeee\nsta f:$8104\nsep #$20\n.a8\nlda #$77\nsta f:$8106\nrep #$20\n.a16\nlda #7\nrtl",
            0x050000,
        );
        options.imports.push(AssemblyImport {
            symbol: symbol.0,
            signature: external.signature.0,
            abi: abi::generated::ABI_NAME.into(),
            address: 0x050000,
            size: assembly.len() as u32,
            stack_peak: 0,
            checks_stack: true,
            irq_effect: Default::default(),
        });
        let compiled = prepared.compile(&options).unwrap();
        let work = compiled
            .machine
            .routines
            .iter()
            .find(|m| {
                m.id == prepared
                    .mir
                    .routines
                    .iter()
                    .find(|r| r.name == "Work" || r.name.starts_with("M_TEST_WORK_"))
                    .unwrap()
                    .id
            })
            .unwrap();
        assert!(
            proof::placement_summary(&work.code)
                .unwrap()
                .unwrap()
                .call_segments
                > 0,
            "{kind}"
        );
        assert!(matches!(
            work.frame.temps[&TempId(0)],
            actionc::mir65816::emit::Location::Stack(_)
        ));
        for mask in [0, 4] {
            for at in [0x21ffffu32, 0x32fffe] {
                let mut h = Harness::new(&compiled.image, &caller(compiled.image.entry), mask);
                h.bus.map(0x050000, &assembly, false);
                for start in [0x8100, 0x8300, at] {
                    h.bus.map(start - 1, &[0xa5; 12], true);
                }
                h.bus.ram[0x8104..0x8107].copy_from_slice(&at.to_le_bytes()[..3]);
                h.bus.ram[at as usize + 2..at as usize + 4]
                    .copy_from_slice(&0xabcd_u16.to_le_bytes());
                assert!(
                    h.cpu
                        .run_until(
                            &mut h.bus,
                            2_000_000,
                            |_| Default::default(),
                            |cpu| cpu.is_stopped()
                        )
                        .unwrap_or_else(|e| panic!(
                            "{kind} pc={:x} {:?}: {e}",
                            h.cpu.pc(),
                            h.cpu.registers()
                        ))
                );
                h.guards(mask);
                // The original record field changes, but p is the immutable
                // earlier capture; reestablishment must not read root.next.
                assert_eq!(
                    h.bus.value(0x8104, 3),
                    if kind == "helper" { at } else { 0x77eeee }
                );
                assert_eq!(h.bus.value(at, 1), 11, "{kind}");
                assert_eq!(
                    h.bus.value(at + 2, 2),
                    if kind == "helper" { 0x369c } else { 0xbeef },
                    "{kind}"
                );
                assert_eq!(
                    h.bus.value(0x8302, 2),
                    if kind == "helper" { 0x369c } else { 7 },
                    "{kind}"
                );
                assert_eq!(h.bus.ram[at as usize + 1], 0xa5);
                assert_eq!(h.bus.ram[at as usize + 7], 0xa5);
            }
        }
    }
}

#[test]
fn nested_loop_residence_survives_every_enabled_irq_site_nmi_and_task_domains() {
    use actionc_vm::native65816::Inputs;
    use std::collections::BTreeSet;
    use support::context::*;
    let source = "MODULE TEST PUBLIC EXTERNAL PROC Yield()\n\
VOLATILE BYTE irqAck=$7800 CARD taskA=$7000,taskB=$7002 BYTE current\n\
TYPE Cell=[BYTE flag CARD value Cell POINTER next]\n\
TYPE Job=[Cell POINTER item BYTE done CARD result BYTE POINTER peer]\n\
CARD irqResult\n\
CARD FUNC Form(Cell POINTER root) Cell POINTER p CARD sum BYTE i,j\n\
 p=root.next sum=0 i=0 WHILE i<2 DO j=0 WHILE j<1 DO\n\
 sum==+p.value root.value=p.value root.value=p.value root.value=p.value\n\
 p=p.next j==+1 OD i==+1 OD RETURN(sum)\n\
CARD FUNC Dispatch(CARD saved BYTE reason) irqAck=1 irqResult=Form(Cell POINTER($9300))\n\
 IF current=0 THEN taskA=saved current=1 RETURN(taskB) FI\n\
 taskB=saved current=0 RETURN(taskA)\n\
PROC Task(Job POINTER work) work.result=Form(work.item) work.done=1\n\
 WHILE work.peer^=0 DO Yield() OD RETURN PROC Main() RETURN ENDMODULE\n";
    for optimize in [false, true] {
        let prepared = prepare(source, optimize);
        // Yield is linked by the independent context harness below; verify the
        // common plan directly without a provisional external import.
        let machine = actionc::mir65816::emit::materialize(&prepared.mir).unwrap();
        if optimize {
            assert!(machine.routines.iter().any(|r| {
                proof::placement_summary(&r.code)
                    .unwrap()
                    .is_some_and(|s| s.loop_homes > 0)
            }));
        }
        let mut h =
            ContextHarness::from_prepared(source, optimize, "Task", &[0x7100, 0x7120], prepared);
        for (job, root, peer) in [
            (0x7100usize, 0x9100u32, 0x7123u32),
            (0x7120, 0x9200, 0x7103),
        ] {
            h.bus.ram[job..job + 3].copy_from_slice(&root.to_le_bytes()[..3]);
            h.bus.ram[job + 6..job + 9].copy_from_slice(&peer.to_le_bytes()[..3]);
        }
        for (root, first, second, a, b) in [
            (0x9100usize, 0x21ffffu32, 0x44fffeu32, 65535u16, 1u16),
            (0x9200, 0x32fffe, 0x55ffff, 32767, 128),
            (0x9300, 0x430001, 0x660001, 17, 254),
        ] {
            for at in [root as u32, first, second] {
                h.bus.map(at, &[0xa5; 8], true);
            }
            h.bus.ram[root + 4..root + 7].copy_from_slice(&first.to_le_bytes()[..3]);
            for (at, next, word) in [(first, second, a), (second, first, b)] {
                h.bus.ram[at as usize + 2..at as usize + 4].copy_from_slice(&word.to_le_bytes());
                h.bus.ram[at as usize + 4..at as usize + 7]
                    .copy_from_slice(&next.to_le_bytes()[..3]);
            }
        }
        let form = h
            .image
            .routines
            .iter()
            .find(|r| r.address == context::routine(&h.image, "Form"))
            .unwrap();
        let range = form.address..form.address + form.size;
        let check = |h: &ContextHarness| {
            h.guards();
            assert_eq!(h.bus.value(DONE, 2), 1);
            assert_eq!(h.bus.value(0x7103, 1), 1);
            assert_eq!(h.bus.value(0x7123, 1), 1);
            assert_eq!(h.bus.value(0x7104, 2), 0);
            assert_eq!(h.bus.value(0x7124, 2), 32895);
            assert_eq!(h.bus.value(context::symbol(&h.image, "irqResult"), 2), 271);
        };
        let mut seen = BTreeSet::new();
        for _ in 0..2_000_000 {
            if h.cpu.is_stopped() {
                break;
            }
            let r = h.cpu.registers();
            if h.cpu.is_instruction_boundary()
                && r.p & 4 == 0
                && [0x2000, 0x2100].contains(&r.d)
                && range.contains(&h.cpu.pc())
                && seen.insert((r.d, h.cpu.pc()))
            {
                let saved_cpu = h.cpu.clone();
                let saved_bus = h.bus.clone();
                let mut pending = true;
                for tick in 0..2_000_000 {
                    if h.cpu.is_stopped() {
                        break;
                    }
                    let writes = h.bus.writes.len();
                    h.tick(Inputs {
                        irq: pending,
                        nmi: tick == 40,
                        ..Default::default()
                    });
                    if h.bus.writes[writes..].iter().any(|&(at, _)| at == IRQ_ACK) {
                        pending = false;
                    }
                }
                check(&h);
                h.cpu = saved_cpu;
                h.bus = saved_bus;
            }
            h.tick(Inputs::default());
        }
        h.guards();
        assert_eq!(h.bus.value(0x7104, 2), 0);
        assert_eq!(h.bus.value(0x7124, 2), 32895);
        assert!(seen.len() >= 40);
    }
}
