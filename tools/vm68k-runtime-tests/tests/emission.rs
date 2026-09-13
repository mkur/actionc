use actionc::{
    lexer,
    mir68k::{self, image, machine::*, *},
    nir, parser, semantic,
    target::TargetId,
};
use actionc_vm68k_tests::{Machine, Memory};

fn program() -> (Mir68kProgram, MachineProgram) {
    let ast = parser::parse(
        &lexer::tokenize("LONGINT result LONGCARD ARRAY table(3)=[1 2] PROC Entry() RETURN")
            .unwrap(),
    )
    .unwrap();
    let model = semantic::analyze_with_options(
        &ast,
        semantic::SemanticOptions::modern().with_target(TargetId::Motorola68000),
    )
    .unwrap();
    let nir = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    let mir = mir68k::lower_program(&nir).unwrap();
    let result = mir.data.iter().find(|d| d.name == "result").unwrap().id;
    let block = MachineBlock {
        id: MachineBlockId(0),
        instructions: vec![
            Instruction::Link {
                register: 6,
                displacement: -4,
            },
            Instruction::Move {
                width: Width::Long,
                source: Ea::Immediate(42),
                destination: Ea::Displacement(6, -4),
            },
            Instruction::Move {
                width: Width::Long,
                source: Ea::Displacement(6, -4),
                destination: Ea::Absolute(Address::new(Target::Data(result))),
            },
            Instruction::Move {
                width: Width::Long,
                source: Ea::Immediate(0),
                destination: Ea::D(0),
            },
            Instruction::Branch {
                condition: Condition::Equal,
                target: Address::new(Target::Block(MachineBlockId(1))),
            },
            Instruction::Jump(Ea::Absolute(Address::new(Target::Block(MachineBlockId(0))))),
        ],
    };
    let r = &mir.routines[0];
    let machine = MachineProgram {
        blocks: vec![
            block,
            MachineBlock {
                id: MachineBlockId(1),
                instructions: vec![Instruction::Unlink(6), Instruction::Rts],
            },
        ],
        routines: vec![MachineRoutine {
            id: r.id,
            entry: MachineBlockId(0),
            frame: r.frame.clone(),
        }],
    };
    (mir, machine)
}

#[test]
fn typed_encoding_and_linking_execute_at_two_origins() {
    for origin in [0x2000, 0x12340] {
        let (mir, machine) = program();
        let image = image::link(&mir, &machine, origin).unwrap();
        let mut memory = Memory::default();
        for segment in &image.segments {
            memory
                .map(
                    segment.address,
                    &segment.bytes,
                    segment.writable,
                    segment.executable,
                )
                .unwrap();
        }
        for zero in &image.zero_fill {
            memory
                .map(
                    zero.address,
                    &vec![0; zero.size as usize],
                    zero.writable,
                    false,
                )
                .unwrap();
        }
        let result = image.symbol("result").unwrap().address().unwrap();
        assert_eq!(memory.bytes(result, 4).unwrap(), [0, 0, 0, 0]);
        let table = image.symbol("table").unwrap();
        let backing = table.array.as_ref().unwrap().backing_address.unwrap();
        assert_eq!(
            memory.bytes(table.address().unwrap(), 4).unwrap(),
            backing.to_be_bytes()
        );
        assert_eq!(
            memory.bytes(backing, 12).unwrap(),
            [0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0]
        );
        let mut vm = Machine::new(memory, image.entry).unwrap();
        vm.run(100).assert_completed();
        assert_eq!(vm.cpu.mem.bytes(result, 4).unwrap(), 42u32.to_be_bytes());
    }
}

#[test]
fn symbol_array_access_follows_the_current_descriptor() {
    let (mir, machine) = program();
    let image = image::link(&mir, &machine, 0x10000).unwrap();
    let mut vm = Machine::from_image(&image).unwrap();
    let table = image.symbol("table").unwrap();
    assert_eq!(vm.read_array(table).unwrap(), [1, 2, 0]);
    let replacement = 0x30000u32;
    vm.cpu.mem.map(replacement, &[0; 12], true, false).unwrap();
    vm.cpu
        .mem
        .write(table.address().unwrap(), &replacement.to_be_bytes())
        .unwrap();
    vm.write_array(table, &[0x12345678, 0x80000000, 9]).unwrap();
    assert_eq!(vm.read_array(table).unwrap(), [0x12345678, 0x80000000, 9]);
    assert_eq!(
        vm.cpu.mem.bytes(replacement, 4).unwrap(),
        [0x12, 0x34, 0x56, 0x78]
    );
    assert_eq!(
        vm.cpu
            .mem
            .bytes(table.array.as_ref().unwrap().backing_address.unwrap(), 12)
            .unwrap(),
        [0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0]
    );
}

#[test]
fn aliases_share_storage_and_selected_byte_relocations_keep_significance() {
    let (mut mir, machine) = program();
    let result = mir.data.iter().find(|d| d.name == "result").unwrap();
    let mut alias = result.clone();
    alias.id = Mir68kDataId::Global(nir::SymbolId(900));
    alias.name = "alias".into();
    alias.placement = Mir68kDataPlacement::Alias {
        target: result.id,
        offset: actionc::target::ByteOffset::ZERO,
    };
    alias.zero_fill = actionc::target::ByteSize::ZERO;
    mir.data.push(alias);
    let table = mir.data.iter_mut().find(|d| d.name == "table").unwrap();
    table.relocations[0].width = actionc::target::ByteSize::ONE;
    table.relocations[0].byte_index = Some(2);
    let image = image::link(&mir, &machine, 0x10000).unwrap();
    assert_eq!(
        image.symbol("result").unwrap().address(),
        image.symbol("alias").unwrap().address()
    );
    let cell = image.symbol("table").unwrap().address().unwrap();
    let segment = image.segments.iter().find(|s| s.address == cell).unwrap();
    assert_eq!(segment.bytes[0], 1);
}

#[test]
fn linker_rejects_bad_origins_relocations_and_overlap() {
    let (mir, machine) = program();
    for origin in [1, 0x0100_0000, 0x00ff_fff0] {
        assert!(image::link(&mir, &machine, origin).is_err());
    }
    let mut bad = machine.clone();
    bad.blocks[0].instructions[0] = Instruction::Jsr(Ea::Absolute(Address::new(Target::Block(
        MachineBlockId(999),
    ))));
    assert!(image::link(&mir, &bad, 0x10000).is_err());
    let mut bad = mir.clone();
    bad.data
        .iter_mut()
        .find(|d| !d.relocations.is_empty())
        .unwrap()
        .relocations[0]
        .addend = i64::MAX;
    assert!(image::link(&bad, &machine, 0x10000).is_err());
    let mut image = image::link(&mir, &machine, 0x10000).unwrap();
    image.zero_fill.push(image.zero_fill[0].clone());
    assert!(image.verify().is_err());
}
