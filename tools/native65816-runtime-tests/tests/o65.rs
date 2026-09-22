mod support;
use actionc::mir65816::{o65 as format, o65::profile::*};
use actionc_vm::native65816::Inputs;
use support::{o65 as native, *};

#[test]
fn relocated_word_comparisons_materialize_both_boolean_outcomes() {
    let source = r#"
CARD a,b
BYTE ARRAY output(8)
PROC Main()
 output(0)=(a=b) output(1)=(a#b) output(2)=(a<b)
 output(3)=(a<=b) output(4)=(a>b) output(5)=(a>=b)
 output(6)=(a<CARD($8000)) output(7)=(CARD($8000)<b)
RETURN
"#;
    for optimize in [false, true] {
        let (bytes, templates) = forwarding::o65(source, optimize);
        for variant in 0..2 {
            let placement = native::placement(&bytes, variant, vec![native::fault(variant)]);
            let image = format::relocate(&bytes, &placement).unwrap();
            let caller = caller(image.entry());
            for (a, b) in [(0u16, 0u16), (0xffff, 1), (0x8000, 0x7fff), (0, 0xffff)] {
                for mask in [0, 4] {
                    let mut h = Harness::new_o65(&image, &caller, mask);
                    h.bus.forwarded_words = forwarding::relocated(&templates, &image);
                    for (name, value) in [("a", a), ("b", b)] {
                        let at = native::object(&image, name) as usize;
                        h.bus.ram[at..at + 2].copy_from_slice(&value.to_le_bytes());
                    }
                    h.run();
                    h.guards(mask);
                    let output = native::object(&image, "output") as usize;
                    assert_eq!(
                        &h.bus.ram[output..output + 8],
                        [
                            a == b,
                            a != b,
                            a < b,
                            a <= b,
                            a > b,
                            a >= b,
                            a < 0x8000,
                            0x8000 < b
                        ]
                        .map(u8::from)
                    );
                    native::record(
                        "word-comparisons",
                        optimize,
                        &bytes,
                        &placement,
                        &image,
                        h.cpu.cycles(),
                        None,
                    );
                }
            }
        }
    }
}

#[test]
fn relocated_fused_branches_consume_flags_and_fix_both_edge_transfers() {
    let source = r#"
CARD a,b
BYTE ARRAY output(8)
PROC Main()
 IF a=b THEN output(0)=1 ELSE output(0)=0 FI
 IF a#b THEN output(1)=1 ELSE output(1)=0 FI
 IF a<b THEN output(2)=1 ELSE output(2)=0 FI
 IF a<=b THEN output(3)=1 ELSE output(3)=0 FI
 IF a>b THEN output(4)=1 ELSE output(4)=0 FI
 IF a>=b THEN output(5)=1 ELSE output(5)=0 FI
 IF a<CARD($8000) THEN output(6)=1 ELSE output(6)=0 FI
 IF CARD($8000)<b THEN output(7)=1 ELSE output(7)=0 FI
RETURN
"#;
    for optimize in [false, true] {
        let (bytes, templates) = forwarding::o65(source, optimize);
        for variant in 0..2 {
            let placement = native::placement(&bytes, variant, vec![native::fault(variant)]);
            let image = format::relocate(&bytes, &placement).unwrap();
            let caller = caller(image.entry());
            for (a, b) in [(0u16, 0u16), (0xffff, 1), (0x8000, 0x7fff), (0, 0xffff)] {
                for mask in [0, 4] {
                    let mut h = Harness::new_o65(&image, &caller, mask);
                    h.bus.forwarded_words = forwarding::relocated(&templates, &image);
                    for (name, value) in [("a", a), ("b", b)] {
                        let at = native::object(&image, name) as usize;
                        h.bus.ram[at..at + 2].copy_from_slice(&value.to_le_bytes());
                    }
                    let mut reached = 0;
                    for _ in 0..100_000 {
                        if h.cpu.is_stopped() {
                            break;
                        }
                        if h.cpu.is_instruction_boundary() {
                            for r in &image.profile().routines {
                                let at = image.routine_address(r);
                                if let Some(w) =
                                    comparison::fused_in_range(&h.cpu, &h.bus, at..at + r.size)
                                {
                                    assert!(
                                        w.targets.iter().all(|pc| (at..at + r.size).contains(pc))
                                    );
                                    assert_eq!(
                                        (w.edges[0].len(), w.edges[1].len()),
                                        (1, 1),
                                        "relocated false jump and true REP/fallthrough remain checked"
                                    );
                                    reached += 1;
                                }
                            }
                        }
                        h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                    }
                    assert!(h.cpu.is_stopped());
                    assert_eq!(reached, 8);
                    h.guards(mask);
                    let output = native::object(&image, "output") as usize;
                    assert_eq!(
                        &h.bus.ram[output..output + 8],
                        [
                            a == b,
                            a != b,
                            a < b,
                            a <= b,
                            a > b,
                            a >= b,
                            a < 0x8000,
                            0x8000 < b
                        ]
                        .map(u8::from)
                    );
                    native::record(
                        "fused-branches",
                        optimize,
                        &bytes,
                        &placement,
                        &image,
                        h.cpu.cycles(),
                        None,
                    );
                }
            }
        }
    }
}

#[test]
fn serialized_images_execute_bank_crossing_data_and_indirect_calls() {
    let source = r#"
CARD input,result,initial=[7]
BYTE ARRAY bytes=[3 5 11 13]
CARD FUNC Add(CARD n) RETURN(n+1)
PROC Main()
 CARD FUNC POINTER cb(CARD n)
 cb=@Add result=cb(input)+initial+CARD(bytes(2))
RETURN
"#;
    for optimize in [false, true] {
        let bytes = native::compile(source, optimize, vec![]);
        for variant in 0..2 {
            let placement = native::placement(&bytes, variant, vec![native::fault(variant)]);
            let image = format::relocate(&bytes, &placement).unwrap();
            for mask in [0, 4] {
                let mut h = Harness::new_o65(&image, &caller(image.entry()), mask);
                for z in image.zero_fill() {
                    assert!(
                        h.bus.ram[z.address as usize..(z.address + z.size) as usize]
                            .iter()
                            .all(|b| *b == 0)
                    );
                }
                let input = native::object(&image, "input");
                h.bus.ram[input as usize..input as usize + 2].copy_from_slice(&13u16.to_le_bytes());
                let mut low = h.cpu.registers().s;
                for _ in 0..2_000_000 {
                    if h.cpu.is_stopped() {
                        break;
                    }
                    h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                    low = low.min(h.cpu.registers().s);
                }
                assert!(h.cpu.is_stopped());
                h.guards(mask);
                assert_eq!(h.bus.value(native::object(&image, "result"), 2), 32);
                native::record(
                    "pointers",
                    optimize,
                    &bytes,
                    &placement,
                    &image,
                    h.cpu.cycles(),
                    Some(u32::from(0x5ff0 - low)),
                );
            }
        }
    }
}
#[test]
fn relocated_imports_preserve_live_values_across_all_scratch_clobbers() {
    let source = r#"
MODULE TEST
PUBLIC EXTERNAL LONGCARD FUNC Smash(LONGCARD ignored)
LONGCARD result
LONGCARD FUNC Work(LONGCARD n)
 LONGCARD FUNC POINTER cb(LONGCARD n)
 cb=@Smash
RETURN((n+LONGCARD($10203))+Smash(n)+cb(n))
PROC Main() result=Work(LONGCARD($55667788)) RETURN
ENDMODULE
"#;
    for optimize in [false, true] {
        let binding = Binding {
            symbol: actionc::nir::runtime_symbol_id("TEST.Smash").0,
            name: "Smash".into(),
            stack_peak: 0,
            checks_stack: true,
            irq_effect: Default::default(),
            domains: 3,
        };
        let bytes = native::compile(source, optimize, vec![binding]);
        let contract = format::inspect(&bytes).unwrap().imports[1].contract.clone();
        for variant in 0..2 {
            let address = if variant == 0 { 0x041000 } else { 0x061000 };
            let smash = assemble(
                "sep #$20\n.a8\nldx #63\nlda #$a7\nagain: sta 0,x\ndex\nbpl again\nrep #$20\n.a16\nldy #$dead\nldx #$1122\nlda #$3344\nrtl",
                address,
            );
            let p = native::placement(
                &bytes,
                variant,
                vec![
                    native::fault(variant),
                    format::Provider {
                        name: "Smash".into(),
                        address,
                        size: smash.len() as u32,
                        contract: contract.clone(),
                    },
                ],
            );
            let image = format::relocate(&bytes, &p).unwrap();
            for mask in [0, 4] {
                let mut h = Harness::new_o65(&image, &caller(image.entry()), mask);
                h.bus.map(address, &smash, false);
                h.run();
                h.guards(mask);
                assert_eq!(
                    h.bus.value(native::object(&image, "result"), 4),
                    0x55667788 + 0x10203 + 2 * 0x11223344
                );
                assert!((0x2000..0x2040).all(|a| h.bus.writes.contains(&(a, 0xa7))));
                native::record(
                    "imports",
                    optimize,
                    &bytes,
                    &p,
                    &image,
                    h.cpu.cycles(),
                    None,
                );
            }
        }
    }
}
#[test]
fn guard_failure_reaches_the_relocated_raw_adapter_before_frame_writes() {
    for optimize in [false, true] {
        let bytes = native::compile(
            "CARD input,result PROC Main() result=input+1 RETURN",
            optimize,
            vec![],
        );
        for variant in 0..2 {
            let p = native::placement(&bytes, variant, vec![native::fault(variant)]);
            let image = format::relocate(&bytes, &p).unwrap();
            let mut h = Harness::new_o65(&image, &caller(image.entry()), 0);
            h.bus.ram[0x2044..0x2046].copy_from_slice(&0x5fecu16.to_le_bytes());
            let mut low = h.cpu.registers().s;
            assert!(
                h.cpu
                    .run_until(
                        &mut h.bus,
                        10000,
                        |_| Inputs::default(),
                        |c| {
                            low = low.min(c.registers().s);
                            c.is_stopped()
                        }
                    )
                    .unwrap()
            );
            assert_eq!(h.cpu.pc(), image.stack_overflow() + 1);
            assert!(h.bus.ram[0x4000..=0x5fec].iter().all(|b| *b == 0xa5));
            native::record(
                "guard",
                optimize,
                &bytes,
                &p,
                &image,
                h.cpu.cycles(),
                Some(u32::from(0x5ff0 - low)),
            );
        }
    }
}
#[test]
fn generated_multi_bank_text_executes_without_wrapping_routines() {
    let mut source = "VOLATILE BYTE counter=$7200 ".to_string();
    for i in 0..8 {
        source.push_str(&format!(
            "PROC Work{i}() {} RETURN ",
            "counter==+1 ".repeat(400)
        ));
    }
    source.push_str("PROC Main() ");
    for i in 0..8 {
        source.push_str(&format!("Work{i}() "));
    }
    source.push_str("RETURN");
    for optimize in [false, true] {
        let bytes = native::compile(&source, optimize, vec![]);
        let profile = format::inspect(&bytes).unwrap();
        assert!(profile.routines.iter().map(|r| r.size).sum::<u32>() > 65536);
        for variant in 0..2 {
            let p = native::placement(&bytes, variant, vec![native::fault(variant)]);
            let image = format::relocate(&bytes, &p).unwrap();
            let mut h = Harness::new_o65(&image, &caller(image.entry()), 0);
            h.run();
            h.guards(0);
            assert_eq!(h.bus.value(0x7200, 1), 128);
            native::record("banks", optimize, &bytes, &p, &image, h.cpu.cycles(), None);
        }
    }
}

#[test]
fn independent_mixed_width_assembly_abi_survives_relocation() {
    for optimize in [false, true] {
        let bindings = [("TEST.AsmMixed", "AsmMixed"), ("TEST.AsmEmpty", "AsmEmpty")]
            .into_iter()
            .map(|(symbol, name)| Binding {
                symbol: actionc::nir::runtime_symbol_id(symbol).0,
                name: name.into(),
                stack_peak: 0,
                checks_stack: true,
                irq_effect: Default::default(),
                domains: 3,
            })
            .collect();
        let bytes = native::compile(&fixture("interop.act"), optimize, bindings);
        let profile = format::inspect(&bytes).unwrap();
        for variant in 0..2 {
            let mut providers = vec![native::fault(variant)];
            for (name, address, size) in
                [("AsmMixed", 0x041000, 0x100), ("AsmEmpty", 0x041100, 0x30)]
            {
                providers.push(format::Provider {
                    name: name.into(),
                    address,
                    size,
                    contract: profile
                        .imports
                        .iter()
                        .find(|i| i.name == name)
                        .unwrap()
                        .contract
                        .clone(),
                });
            }
            let p = native::placement(&bytes, variant, providers);
            let image = format::relocate(&bytes, &p).unwrap();
            let mut defines = format!("ACTION_MAIN=${:06x}\n", image.entry());
            for (name, label) in [
                ("ActionMixed", "ACTION_MIXED"),
                ("EchoByte", "ECHO_BYTE"),
                ("EchoWord", "ECHO_WORD"),
                ("EchoAddress", "ECHO_ADDRESS"),
                ("EchoSize", "ECHO_SIZE"),
                ("EchoLong", "ECHO_LONG"),
                ("EchoPointer", "ECHO_POINTER"),
            ] {
                defines.push_str(&format!("{label}=${:06x}\n", native::routine(&image, name)));
            }
            let caller = assemble(&(defines + &fixture("interop.s")), 0x040000);
            for mask in [0, 4] {
                let mut h = Harness::new_o65(&image, &caller, mask);
                h.bus.map(0xab789a, &[0x34], true);
                h.run();
                h.guards(mask);
                assert_eq!(
                    &h.bus.ram[0x7100..0x710d],
                    &[
                        0x12, 0, 0x56, 0x34, 0x9a, 0x78, 0xab, 0, 0x12, 0xf0, 0xde, 0xbc, 0
                    ]
                );
                for (address, width, expected) in [
                    (0x7000, 4, 0xbcdef012u32.wrapping_add(0x12 + 0x3456 + 0x34)),
                    (0x7004, 2, 0xb7),
                    (0x7006, 2, 0xfedc),
                    (0x7008, 4, 0x834567),
                    (0x700c, 4, 0xefabcd),
                    (0x7010, 4, 0xab789a),
                    (0x7014, 4, 0x89abcdef),
                    (0x7200, 3, 1),
                ] {
                    assert_eq!(h.bus.value(address, width), expected);
                }
                assert_eq!(
                    h.bus.value(native::object(&image, "importedResult"), 4),
                    0x89abcdef
                );
                native::record(
                    "interop",
                    optimize,
                    &bytes,
                    &p,
                    &image,
                    h.cpu.cycles(),
                    None,
                );
            }
        }
    }
}
fn context_image(
    bytes: &[u8],
    variant: usize,
) -> (
    support::context::ContextHarness<format::RelocatedImage>,
    format::Placement,
) {
    use support::context::*;
    let provisional = runtime(0x100000);
    let profile = format::inspect(bytes).unwrap();
    let mut providers = vec![];
    for import in &profile.imports {
        let (address, size) = if import.name == OVERFLOW {
            (FAULT, 2)
        } else {
            let a = provisional.symbols[&import.name];
            let e = provisional
                .symbols
                .values()
                .copied()
                .filter(|v| *v > a)
                .min()
                .unwrap();
            (a, e - a)
        };
        providers.push(format::Provider {
            name: import.name.clone(),
            address,
            size,
            contract: import.contract.clone(),
        });
    }
    let p = native::placement(bytes, variant, providers);
    let image = format::relocate(bytes, &p).unwrap();
    let runtime = runtime(native::routine(&image, "Dispatch"));
    assert_eq!(runtime.symbols, provisional.symbols);
    let mut h = ContextHarness::from_loaded(image, runtime, "Task", &[0x7100, 0x7120]);
    for (i, seed) in [13u16, 41].into_iter().enumerate() {
        let at = 0x7100 + i * 0x20;
        h.bus.ram[at..at + 2].copy_from_slice(&seed.to_le_bytes());
        h.bus.ram[at + 5..at + 8]
            .copy_from_slice(&(0x7104u32 + (1 - i as u32) * 0x20).to_le_bytes()[..3]);
        let buffer = 0x12fffc + i as u32 * 0x10000;
        h.bus.ram[at + 8..at + 11].copy_from_slice(&buffer.to_le_bytes()[..3]);
        h.bus
            .map(buffer, &[10, 20, 30, 40, 50, 60, 70, 80, 90, 100], true);
    }
    (h, p)
}
fn run_context(
    h: &mut support::context::ContextHarness<format::RelocatedImage>,
    seed: Option<u64>,
    inject: Option<u32>,
) -> std::collections::BTreeSet<u32> {
    use support::context::*;
    let mut rng = seed.unwrap_or(1);
    let mut pending = false;
    let mut injected = false;
    let mut cooldown = 0;
    let mut addresses = std::collections::BTreeSet::new();
    for _ in 0..2_000_000 {
        if h.cpu.is_stopped() {
            assert_eq!(h.bus.value(DONE, 2), 1);
            h.guards();
            for (i, expected) in [55, 111].into_iter().enumerate() {
                assert_eq!(h.bus.value(0x7102 + i as u32 * 0x20, 2), expected);
                assert_eq!(h.bus.value(0x7104 + i as u32 * 0x20, 1), 1);
                let buffer = 0x12fffc + i * 0x10000;
                assert_eq!(
                    &h.bus.ram[buffer..buffer + 10],
                    &[10, 10, 20, 0, 0, 50, 60, 70, 90, 100]
                );
            }
            assert!(h.bus.value(native::object(&h.image, "dispatches"), 2) >= 2);
            if inject.is_some() {
                assert!(injected);
            }
            return addresses;
        }
        let mut nmi = false;
        if h.cpu.is_instruction_boundary() {
            let pc = h.cpu.pc();
            if h.cpu.registers().p & 4 == 0 {
                if h.image.segments().iter().any(|s| {
                    s.executable && pc >= s.address && pc < s.address + s.bytes.len() as u32
                }) {
                    addresses.insert(pc);
                }
                if !injected && inject == Some(pc) {
                    pending = true;
                    injected = true;
                }
            }
            if seed.is_some() {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                if rng & 31 == 0 && h.cpu.registers().p & 4 == 0 {
                    pending = true;
                }
                if rng & 127 == 1 && h.cpu.cycles() >= cooldown {
                    nmi = true;
                    cooldown = h.cpu.cycles() + 250;
                }
            }
        }
        let before = h.bus.writes.len();
        h.tick(Inputs {
            irq: pending,
            nmi,
            ..Default::default()
        });
        if h.bus.writes[before..].iter().any(|&(a, _)| a == IRQ_ACK) {
            pending = false;
        }
    }
    panic!("o65 context budget");
}
#[test]
fn relocated_tasks_preserve_domains_under_irq_nmi_and_instruction_injection() {
    use actionc::mir65816::image::IrqEffect;
    for optimize in [false, true] {
        let bindings = [
            ("TEST.Yield", "__a816_yield_v1", 1, IrqEffect::Preserve, 1),
            (
                "TEST.SaveIRQ",
                "__a816_irq_save_disable_v1",
                1,
                IrqEffect::SaveDisable,
                3,
            ),
            (
                "TEST.RestoreIRQ",
                "__a816_irq_restore_v1",
                0,
                IrqEffect::Restore,
                3,
            ),
        ]
        .into_iter()
        .map(|(symbol, name, stack_peak, irq_effect, domains)| Binding {
            symbol: actionc::nir::runtime_symbol_id(symbol).0,
            name: name.into(),
            stack_peak,
            checks_stack: true,
            irq_effect,
            domains,
        })
        .collect();
        let bytes = native::compile(&fixture("preemption.act"), optimize, bindings);
        for variant in 0..2 {
            let (mut h, p) = context_image(&bytes, variant);
            let addresses = run_context(&mut h, None, None);
            native::record(
                "contexts",
                optimize,
                &bytes,
                &p,
                &h.image,
                h.cpu.cycles(),
                None,
            );
            for seed in [0x81620260916, 0x5eedcafe] {
                let (mut h, _) = context_image(&bytes, variant);
                run_context(&mut h, Some(seed), None);
            }
            for address in addresses
                .iter()
                .step_by((addresses.len() / 6).max(1))
                .take(6)
            {
                let (mut h, _) = context_image(&bytes, variant);
                run_context(&mut h, None, Some(*address));
            }
        }
    }
}

#[test]
fn initialized_split_addresses_wide_containers_and_aliases_execute() {
    use actionc::{
        mir65816::*,
        target::{ByteOffset, ByteSize, TargetLayout},
    };
    let source = "BYTE ARRAY buffer=[1 2 3 4 5 6 0 0] BYTE ARRAY pieces=[0 0 0] LONGCARD address=[0],result BYTE alias,a,b,c,tail PROC Main() result=address a=pieces(0) b=pieces(1) c=pieces(2) alias=99 tail=buffer(7) RETURN";
    for optimize in [false, true] {
        let bytes = {
            let mut prepared = prepare(source, optimize);
            let m = &mut prepared.mir;
            let entry = m.routines.iter().find(|r| r.entry.program).unwrap().id;
            let buffer = m
                .data
                .iter()
                .find(|d| {
                    d.name.to_ascii_lowercase().contains("buffer")
                        && matches!(d.id, Mir65816DataId::ArrayBacking(_))
                })
                .unwrap()
                .id;
            let target = match buffer {
                Mir65816DataId::Global(id) => {
                    Mir65816RelocationTarget::Data(actionc::nir::NirStorageId::Global(id))
                }
                Mir65816DataId::ArrayBacking(id) => Mir65816RelocationTarget::ArrayBacking(id),
                _ => panic!("unexpected backing identity"),
            };
            for d in &mut m.data {
                if d.id == buffer {
                    // Exercise a partial initialized object with a contiguous
                    // zero tail, independently of frontend initializer packing.
                    d.bytes = vec![1, 2, 3, 4, 5, 6];
                    d.zero_fill = ByteSize::new(2);
                }
                if d.name == "address" {
                    d.relocations = vec![Mir65816Relocation {
                        offset: ByteOffset::ZERO,
                        byte_index: None,
                        width: ByteSize::new(4),
                        address_space: TargetLayout::CODE_ADDRESS_SPACE,
                        target: Mir65816RelocationTarget::Code(entry),
                        addend: 0,
                    }];
                }
                if d.name.to_ascii_lowercase().contains("pieces")
                    && matches!(d.id, Mir65816DataId::ArrayBacking(_))
                {
                    d.relocations = (0..3)
                        .map(|b| Mir65816Relocation {
                            offset: ByteOffset::new(b),
                            byte_index: Some(b as u8),
                            width: ByteSize::ONE,
                            address_space: TargetLayout::DATA_ADDRESS_SPACE,
                            target,
                            addend: 7,
                        })
                        .collect();
                }
                if d.name == "alias" {
                    d.placement = Mir65816DataPlacement::Alias {
                        target: buffer,
                        offset: ByteOffset::new(3),
                    };
                }
            }
            prepared.compile_o65(&Default::default()).unwrap().bytes
        };
        for variant in 0..2 {
            let p = native::placement(&bytes, variant, vec![native::fault(variant)]);
            let image = format::relocate(&bytes, &p).unwrap();
            let mut h = Harness::new_o65(&image, &caller(image.entry()), 0);
            h.run();
            h.guards(0);
            assert_eq!(
                h.bus.value(native::object(&image, "result"), 4),
                image.entry()
            );
            let expected = (native::object(&image, "buffer.__backing") + 7).to_le_bytes();
            for (name, value) in ["a", "b", "c"].into_iter().zip(expected) {
                assert_eq!(
                    h.bus.value(native::object(&image, name), 1),
                    u32::from(value)
                );
            }
            let base = native::object(&image, "buffer.__backing");
            assert_eq!(h.bus.value(native::object(&image, "tail"), 1), 0);
            assert_eq!(
                &h.bus.ram[base as usize..base as usize + 8],
                &[1, 2, 3, 99, 5, 6, 0, 0]
            );
            let again = format::relocate(&bytes, &p).unwrap();
            let fresh = Harness::new_o65(&again, &caller(again.entry()), 0);
            assert_eq!(
                &fresh.bus.ram[base as usize..base as usize + 8],
                &[1, 2, 3, 4, 5, 6, 0, 0]
            );
            native::record(
                "static-addresses",
                optimize,
                &bytes,
                &p,
                &image,
                h.cpu.cycles(),
                None,
            );
        }
    }
}

#[test]
fn relocated_nonempty_word_edges_preserve_cycles_and_both_conditional_arms() {
    for optimize in [false, true] {
        for rotation in [false, true] {
            // Drop compiler objects before reading/relocating the serialized file.
            let (bytes, templates) = {
                let p = if rotation {
                    edges::rotation(optimize, false, false)
                } else {
                    edges::program(optimize, false)
                };
                let m = actionc::mir65816::emit::materialize(&p.mir).unwrap();
                let templates = forwarding::index(&p.mir, &m, |id| {
                    0x10000 * (1 + m.routines.iter().position(|r| r.id == id).unwrap() as u32)
                });
                (p.compile_o65(&Default::default()).unwrap().bytes, templates)
            };
            for variant in 0..2 {
                let placement = native::placement(&bytes, variant, vec![native::fault(variant)]);
                let image = format::relocate(&bytes, &placement).unwrap();
                let mut reached = std::collections::BTreeSet::new();
                for (a, b) in [(0u16, 0xffffu16), (0xffff, 0), (0x8000, 0x7fff)] {
                    for mask in [0, 4] {
                        let mut h = Harness::new_o65(&image, &caller(image.entry()), mask);
                        h.bus.forwarded_words = forwarding::relocated(&templates, &image);
                        h.bus.ram[0x7100..0x7102].copy_from_slice(&a.to_le_bytes());
                        h.bus.ram[0x7102..0x7104].copy_from_slice(&b.to_le_bytes());
                        let mut count = 0;
                        for _ in 0..100000 {
                            if h.cpu.is_stopped() {
                                break;
                            }
                            if h.cpu.is_instruction_boundary()
                                && h.cpu.registers().p & 0x30 == 0
                                && matches!(h.bus.ram[h.cpu.pc() as usize], 0xa3 | 0xa9)
                            {
                                for r in &image.profile().routines {
                                    let at = image.routine_address(r);
                                    if let Some(w) =
                                        word_edge::decode(&h.bus, h.cpu.pc(), at..at + r.size)
                                    {
                                        count += 1;
                                        reached.insert((h.cpu.pc(), w.target, w.moves.len()));
                                    }
                                }
                            }
                            h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                        }
                        assert!(h.cpu.is_stopped());
                        h.guards(mask);
                        assert_eq!(count, if rotation { 5 } else { 1 });
                        let expected = if rotation {
                            a.wrapping_sub(b).wrapping_add(a)
                        } else {
                            a.max(b).wrapping_sub(a.min(b))
                        };
                        assert_eq!(h.bus.value(0x7200, 2), u32::from(expected));
                        native::record(
                            if rotation {
                                "word-rotation"
                            } else {
                                "word-edges"
                            },
                            optimize,
                            &bytes,
                            &placement,
                            &image,
                            h.cpu.cycles(),
                            None,
                        );
                    }
                }
                assert_eq!(reached.len(), if rotation { 3 } else { 2 });
                if let Ok(directory) = std::env::var("A816_QUALIFICATION_DIR") {
                    std::fs::write(std::path::Path::new(&directory).join(format!("word-edge-o65-{optimize}-{rotation}-{variant}.json")),serde_json::to_vec_pretty(&serde_json::json!({"sites":reached,"columns":["load","target","words"]})).unwrap()).unwrap();
                }
            }
        }
    }
}

#[test]
fn relocated_direct_word_edges_cover_immediates_branches_and_backedges() {
    for optimize in [false, true] {
        for ordinary in [false, true] {
            let (bytes, templates) = {
                let p = edges::single(optimize, ordinary);
                let c = p.compile(&layout()).unwrap();
                let sites = word_edge::index(&p.mir, &c.machine, |id| {
                    c.image
                        .routines
                        .iter()
                        .find(|r| r.id == id.0)
                        .unwrap()
                        .address
                });
                let templates: Vec<_> = sites
                    .values()
                    .map(|s| {
                        let r = c
                            .image
                            .routines
                            .iter()
                            .find(|r| r.address == s.range.start)
                            .unwrap();
                        (r.id, s.clone())
                    })
                    .collect();
                (p.compile_o65(&Default::default()).unwrap().bytes, templates)
            };
            assert_eq!(templates.len(), 5);
            for variant in 0..2 {
                let placement = native::placement(&bytes, variant, vec![native::fault(variant)]);
                let image = format::relocate(&bytes, &placement).unwrap();
                let mut sites = word_edge::Index::new();
                for (id, template) in &templates {
                    let r = image
                        .profile()
                        .routines
                        .iter()
                        .find(|r| r.id == *id)
                        .unwrap();
                    let base = image.routine_address(r);
                    let old = template.range.start;
                    let mut s = template.clone();
                    assert_eq!(r.size, s.range.end - old);
                    s.range = base..base + r.size;
                    s.load = base + s.load - old;
                    s.jump = base + s.jump - old;
                    s.target = base + s.target - old;
                    assert!(s.direct);
                    sites.insert(s.load, s);
                }
                let mut reached = std::collections::BTreeSet::new();
                for (a, b) in [(0u16, 0xffffu16), (0xffff, 0), (0x8000, 0x7fff)] {
                    for mask in [0, 4] {
                        let mut h = Harness::new_o65(&image, &caller(image.entry()), mask);
                        h.bus.single_word_edges = sites.clone();
                        h.bus.ram[0x7100..0x7102].copy_from_slice(&a.to_le_bytes());
                        h.bus.ram[0x7102..0x7104].copy_from_slice(&b.to_le_bytes());
                        let mut count = 0;
                        for _ in 0..100000 {
                            if h.cpu.is_stopped() {
                                break;
                            }
                            if h.cpu.is_instruction_boundary() {
                                if let Some(site) = sites.get(&h.cpu.pc()) {
                                    let w =
                                        word_edge::decode(&h.bus, h.cpu.pc(), site.range.clone())
                                            .unwrap();
                                    assert!(
                                        w.form == word_edge::Form::Direct
                                            && w.moves.len() == 1
                                            && w.sites.len()
                                                == if site.fallthrough { 2 } else { 3 }
                                    );
                                    count += 1;
                                    reached.insert((site.load, w.target));
                                    let saved = h.bus.ram[0x4000..0x6000].to_vec();
                                    let dest = u32::from(h.cpu.registers().s)
                                        + u32::from(site.destination);
                                    let cycles = h.cpu.cycles();
                                    h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                                    while !h.cpu.is_instruction_boundary() || h.cpu.pc() != w.target
                                    {
                                        h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                                    }
                                    assert_eq!(
                                        h.cpu.cycles() - cycles,
                                        (if site.source.0 { 14 } else { 12 })
                                            - if site.fallthrough { 4 } else { 0 }
                                    );
                                    for (i, &byte) in saved.iter().enumerate() {
                                        let at = 0x4000 + i as u32;
                                        if !(dest..dest + 2).contains(&at) {
                                            assert_eq!(h.bus.ram[at as usize], byte);
                                        }
                                    }
                                    continue;
                                }
                            }
                            h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                        }
                        assert!(h.cpu.is_stopped());
                        assert_eq!(count, 6);
                        h.guards(mask);
                        assert_eq!(h.bus.value(0x7200, 2), u32::from(a.min(b).wrapping_add(a)));
                        native::record(
                            if ordinary {
                                "direct-word-ordinary"
                            } else {
                                "direct-word-fused"
                            },
                            optimize,
                            &bytes,
                            &placement,
                            &image,
                            h.cpu.cycles(),
                            None,
                        );
                    }
                }
                assert_eq!(reached.len(), 5);
                if let Ok(directory) = std::env::var("A816_QUALIFICATION_DIR") {
                    std::fs::write(
                        std::path::Path::new(&directory).join(format!(
                            "direct-word-o65-{optimize}-{ordinary}-{variant}.json"
                        )),
                        serde_json::to_vec_pretty(
                            &serde_json::json!({"sites":reached,"columns":["load","target"]}),
                        )
                        .unwrap(),
                    )
                    .unwrap();
                }
            }
        }
    }
}

#[test]
fn relocated_accumulator_forwarding_keeps_words_flags_and_exact_volatile_traces() {
    use actionc_vm::native65816::Access;
    use std::collections::BTreeSet;
    let source = include_str!("fixtures/accumulator_forwarding.act").replace("\r\n", "\n");
    for optimize in [false, true] {
        let (bytes, templates) = forwarding::o65(&source, optimize);
        for variant in 0..2 {
            let placement = native::placement(&bytes, variant, vec![native::fault(variant)]);
            let image = format::relocate(&bytes, &placement).unwrap();
            let sites = forwarding::relocated(&templates, &image);
            let mut seen = BTreeSet::new();
            for (a, b) in [
                (0u16, 1u16),
                (1, 0),
                (0x7fff, 0x8000),
                (0x8000, 0x7fff),
                (0xffff, 0xffff),
            ] {
                for mask in [0, 4] {
                    let mut h = Harness::new_o65(&image, &caller(image.entry()), mask);
                    h.bus.forwarded_words = sites.clone();
                    for (name, value) in [("a", a), ("b", b)] {
                        let at = native::object(&image, name) as usize;
                        h.bus.ram[at..at + 2].copy_from_slice(&value.to_le_bytes());
                    }
                    h.bus.map(0xd000, &[0xff, 0x7f], true);
                    h.bus.watched.extend([0xd000, 0xd001]);
                    for _ in 0..100_000 {
                        if h.cpu.is_stopped() {
                            break;
                        }
                        if let Some(s) = forwarding::reached(&h.cpu, &h.bus) {
                            let r = h.cpu.registers();
                            assert_eq!(
                                u32::from(r.a),
                                h.bus.value(u32::from(r.s) + u32::from(s.slot), 2)
                            );
                            assert_eq!(
                                r.p & 0x82,
                                if r.a == 0 { 2 } else { 0 }
                                    | if r.a & 0x8000 != 0 { 0x80 } else { 0 }
                            );
                            seen.insert(s.start);
                        }
                        h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                    }
                    assert!(h.cpu.is_stopped());
                    h.guards(mask);
                    for (name, value) in [
                        ("result", a.wrapping_sub(1)),
                        ("maximum", a.max(b)),
                        ("stored", a.wrapping_sub(1)),
                        ("before", 0x7fff),
                        ("after", 0x8000),
                    ] {
                        assert_eq!(
                            h.bus.value(native::object(&image, name), 2),
                            u32::from(value)
                        );
                    }
                    assert_eq!(
                        h.bus
                            .trace
                            .iter()
                            .map(|&(_, a, k)| (a, k))
                            .collect::<Vec<_>>(),
                        vec![
                            (0xd000, Access::Read),
                            (0xd001, Access::Read),
                            (0xd000, Access::Write(0)),
                            (0xd001, Access::Write(0x80)),
                            (0xd000, Access::Read),
                            (0xd001, Access::Read)
                        ]
                    );
                    native::record(
                        "accumulator-forwarding",
                        optimize,
                        &bytes,
                        &placement,
                        &image,
                        h.cpu.cycles(),
                        None,
                    );
                }
            }
            assert_eq!(seen, sites.keys().copied().collect());
            if let Ok(directory) = std::env::var("A816_QUALIFICATION_DIR") {
                std::fs::write(
                    std::path::Path::new(&directory)
                        .join(format!("accumulator-o65-{optimize}-{variant}.json")),
                    serde_json::to_vec_pretty(
                        &serde_json::json!({"reached":seen,"executions":10,"both_masks":true}),
                    )
                    .unwrap(),
                )
                .unwrap();
            }
        }
    }
}
