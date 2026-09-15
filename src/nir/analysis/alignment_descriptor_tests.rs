// Included in alignment's test module to share the verified source adapter.
const DESCRIPTOR_SOURCE: &str = "\
LONGCARD ARRAY data(8), grid(2,2)
LONGCARD answer
BYTE flag
PROC Touch() RETURN
PROC Main()
  answer=grid(0,0)
  grid=data
  answer=grid(0,0)
  grid=@data(0)+1
  answer=grid(0,0)
RETURN
";

fn descriptor_load_alignments(program: &NirProgram) -> Vec<u32> {
    let analysis = analyze_alignment(program).unwrap();
    let routine = program.routines.iter().find(|r| r.name == "Main").unwrap();
    let grid = program
        .globals
        .iter()
        .find(|g| g.name == "grid")
        .unwrap()
        .id;
    routine
        .blocks
        .iter()
        .flat_map(|block| {
            block.ops.iter().filter_map(|op| {
                if let NirOp::Load { dest, place, ty } = op
                    && direct_storage_id(place) == Some(NirStorageId::Global(grid))
                {
                    Some(
                        analysis
                            .value_alignment(
                                routine.id,
                                block.id,
                                &NirValue::Temp {
                                    id: *dest,
                                    ty: ty.clone(),
                                },
                            )
                            .get(),
                    )
                } else {
                    None
                }
            })
        })
        .collect()
}

#[test]
fn descriptor_runtime_assignments_do_not_retroactively_prove_loads_or_assume_initializers() {
    let program = lower(DESCRIPTOR_SOURCE);
    assert_eq!(descriptor_load_alignments(&program), [1, 2, 1]);
    assert_eq!(
        descriptor_load_alignments(&optimize_program(&program).unwrap()),
        [1, 2, 1]
    );
    let source = DESCRIPTOR_SOURCE.replace("grid=@data(0)+1", "Touch()");
    let program = lower(&source);
    assert_eq!(descriptor_load_alignments(&program), [1, 2, 1]);
    let program =
        lower(&source.replace("PROC Touch() RETURN", "PROC Touch() grid=@data(0)+1 RETURN"));
    assert_eq!(descriptor_load_alignments(&program), [1, 2, 1]);
    let mut program = lower(&source);
    for op in program
        .routines
        .iter_mut()
        .flat_map(|r| &mut r.blocks)
        .flat_map(|b| &mut b.ops)
    {
        if let NirOp::Call { effects, .. } = op {
            effects.memory.writes = NirMemoryAccess::Unknown;
        }
    }
    assert_eq!(descriptor_load_alignments(&program), [1, 2, 1]);
    for (offset, expected) in [(0, 1), (4, 2)] {
        let mut program = lower(&source);
        let grid = program
            .globals
            .iter()
            .find(|g| g.name == "grid")
            .unwrap()
            .id;
        for op in program
            .routines
            .iter_mut()
            .flat_map(|r| &mut r.blocks)
            .flat_map(|b| &mut b.ops)
        {
            if let NirOp::Call { effects, .. } = op {
                effects.memory.writes = NirMemoryAccess::Regions(vec![NirMemoryRegion {
                    kind: NirMemoryRegionKind::Storage(NirStorageId::Global(grid)),
                    offset: ByteOffset::new(offset),
                    size: ByteSize::new(2),
                }]);
            }
        }
        assert_eq!(descriptor_load_alignments(&program), [1, 2, expected]);
    }
    let program = lower(&DESCRIPTOR_SOURCE.replace(
        "grid=@data(0)+1",
        "IF flag THEN grid=data ELSE grid=@data(0)+1 FI",
    ));
    assert_eq!(descriptor_load_alignments(&program), [1, 2, 1]);
}

#[test]
fn descriptor_regions_distinguish_pointer_bytes_size_word_and_backing_writes() {
    for (offset, expected) in [(0, 1), (1, 1), (3, 1), (4, 2)] {
        let mut program = lower(DESCRIPTOR_SOURCE);
        let routine = program
            .routines
            .iter_mut()
            .find(|r| r.name == "Main")
            .unwrap();
        let grid = program
            .globals
            .iter()
            .find(|g| g.name == "grid")
            .unwrap()
            .id;
        let mut stores = 0;
        let temp = TempId(routine.temps.iter().map(|t| t.id.0).max().unwrap() + 1);
        for block in &mut routine.blocks {
            let mut ops = Vec::new();
            for mut op in std::mem::take(&mut block.ops) {
                if let NirOp::Store { place, src, ty } = &mut op
                    && direct_storage_id(place) == Some(NirStorageId::Global(grid))
                {
                    stores += 1;
                    if stores == 2 {
                        let byte = NirType::from_value_with_layout(
                            &crate::semantic::ValueType::fund(crate::ast::FundType::Byte),
                            program.target_layout,
                        );
                        let mut pointer = ty.clone();
                        if let NirTypeKind::Pointer { pointee, .. } = &mut pointer.kind {
                            *pointee = Some(Box::new(byte.kind.clone()));
                        }
                        ops.push(NirOp::AddrOf {
                            dest: temp,
                            ty: pointer.clone(),
                            place: place.clone(),
                        });
                        *place = NirPlace {
                            kind: NirPlaceKind::Field {
                                base: Box::new(NirPlace {
                                    kind: NirPlaceKind::Deref {
                                        addr: NirValue::Temp {
                                            id: temp,
                                            ty: pointer,
                                        },
                                    },
                                    ty: Some(byte.clone()),
                                }),
                                offset: ByteOffset::new(offset),
                                ty: byte.clone(),
                            },
                            ty: Some(byte.clone()),
                        };
                        *src = NirValue::ConstU8(1);
                        *ty = byte;
                    }
                }
                ops.push(op);
            }
            block.ops = ops;
        }
        routine.temps = crate::nir::optimizer::collect_temps(&routine.blocks);
        assert_eq!(stores, 2);
        assert_eq!(
            descriptor_load_alignments(&program),
            [1, 2, expected],
            "offset {offset}"
        );
    }
    let program = lower(&DESCRIPTOR_SOURCE.replace("grid=@data(0)+1", "data(0)=7"));
    assert_eq!(descriptor_load_alignments(&program), [1, 2, 2]);
    let program = lower(&DESCRIPTOR_SOURCE.replace("grid=@data(0)+1", "data(flag)=7"));
    assert_eq!(descriptor_load_alignments(&program), [1, 2, 1]);
}

#[test]
fn invocation_local_descriptor_initialization_proves_only_actual_aligned_backing() {
    for (element, expected) in [("LONGCARD", 2), ("BYTE", 1)] {
        let program = lower(&format!(
            "LONGCARD answer\nPROC Main()\n {element} ARRAY grid(2,2)\n answer=grid(0,0)\nRETURN\n"
        ));
        let analysis = analyze_alignment(&program).unwrap();
        let routine = program.routines.iter().find(|r| r.name == "Main").unwrap();
        let mut accesses = 0;
        for block in &routine.blocks {
            for op in &block.ops {
                if let NirOp::Load {
                    place:
                        NirPlace {
                            kind: NirPlaceKind::Index { base_addr, .. },
                            ..
                        },
                    ..
                } = op
                {
                    accesses += 1;
                    assert_eq!(
                        analysis
                            .value_alignment(routine.id, block.id, base_addr)
                            .get(),
                        expected
                    );
                    assert!(
                        analysis
                            .volatile_value_proof(routine.id, block.id, base_addr)
                            .is_none()
                    );
                }
            }
        }
        assert_eq!(accesses, 1);
    }
}
