use actionc::{
    lexer,
    mir65816::{self, o65, relocation},
    nir, parser, semantic,
    target::TargetId,
};
fn mir(source: &str, optimize: bool) -> mir65816::Mir65816Program {
    let ast = parser::parse(&lexer::tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(
        &ast,
        semantic::SemanticOptions::modern().with_target(TargetId::Wdc65816Native),
    )
    .unwrap();
    let n = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    let n = if optimize {
        nir::optimize_program_with_promotion(&n, nir::NirPromotionPolicy::Native65816).unwrap()
    } else {
        n
    };
    mir65816::lower_program(&n).unwrap()
}
#[test]
fn retains_typed_targets_and_builds_independent_sections() {
    for optimize in [false, true] {
        let program = mir(
            "CARD output,initial=[7] BYTE ARRAY table=[1 2 3] CARD FUNC Add(CARD n) RETURN(n+1) PROC Main() output=Add(initial)+CARD(table(1)) RETURN",
            optimize,
        );
        let machine = mir65816::emit::materialize(&program).unwrap();
        let pending = relocation::collect(&program, &machine).unwrap();
        assert!(
            pending
                .iter()
                .any(|f| matches!(f.target, relocation::Target::StackOverflow))
        );
        assert!(
            pending
                .iter()
                .any(|f| matches!(f.target, relocation::Target::Data(_)))
        );
        let artifact = o65::prepare(&program, &Default::default()).unwrap();
        assert_eq!(artifact.profile().imports[0].name, o65::profile::OVERFLOW);
        assert!(artifact.section_sizes().iter().all(|s| *s > 0));
        assert_eq!(artifact.profile().routines.len(), 2);
        assert!(
            artifact
                .profile()
                .relocations
                .iter()
                .any(|r| matches!(r.target, o65::profile::Reference::Import(0)))
        );
    }
}
#[test]
fn rejects_overlapping_fixups_and_alias_cycles() {
    let mut program = mir("CARD value PROC Main() value=7 RETURN", false);
    let mut machine = mir65816::emit::materialize(&program).unwrap();
    let f = machine.routines[0].code.fixups[0].clone();
    machine.routines[0].code.fixups.push(f);
    assert!(
        relocation::collect(&program, &machine)
            .unwrap_err()
            .contains("overlapping")
    );
    let d = &mut program.data[0];
    d.placement = mir65816::Mir65816DataPlacement::Alias {
        target: d.id,
        offset: actionc::target::ByteOffset::ZERO,
    };
    assert!(o65::prepare(&program, &Default::default()).is_err());
}
#[test]
fn rejects_unsupported_profiles_and_unchecked_bindings() {
    let p = mir("PROC Main() RETURN", false);
    let mut options = o65::Options::default();
    options.profile = "future".into();
    assert!(o65::prepare(&p, &options).unwrap_err().contains("profile"));
    options.profile = o65::profile::ID.into();
    options.imports.push(o65::Binding {
        symbol: 1,
        name: "Host".into(),
        stack_peak: 0,
        checks_stack: false,
        domains: 3,
        irq_effect: Default::default(),
    });
    assert!(
        o65::prepare(&p, &options)
            .unwrap_err()
            .contains("unchecked")
    );
}

fn inspect(bytes: &[u8]) -> serde_json::Value {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "actionc-o65-inspect-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, bytes).unwrap();
    let result = std::process::Command::new("python3")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tools/inspect_o65.py"))
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}
#[test]
fn writes_deterministically_and_matches_independent_decoder() {
    let p = mir("CARD value PROC Main() value=7 RETURN", false);
    let a = o65::prepare(&p, &Default::default()).unwrap();
    let bytes = o65::write(&a).unwrap();
    assert_eq!(bytes, o65::write(&a).unwrap());
    let decoded = inspect(&bytes);
    assert_eq!(decoded["mode"], 0xa202);
    assert_eq!(decoded["width"], 4);
    assert_eq!(decoded["exports"][0]["name"], o65::profile::ENTRY);
    assert_eq!(decoded["imports"][0], o65::profile::OVERFLOW);
    assert_eq!(
        decoded["relocations"].as_array().unwrap().len(),
        a.profile().relocations.len()
    );
    let foreign = inspect(include_bytes!("../fixtures/o65/vasm_native.o65"));
    let kinds = foreign["relocations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["kind"].as_u64().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(kinds, [0x20, 0x40, 0x80, 0xa0, 0xc0].into());
}
#[test]
fn wide_fields_and_relocation_gap_boundaries_are_exact() {
    use o65::{
        profile::*,
        wire::{Export, File},
    };
    for size in [65535, 65536, 65537] {
        let offsets = [0, 1, 255, 510, 1018, 65530];
        let file = File {
            mode: o65::wire::MODE,
            bases: [0x18000, 0, 0, 0],
            lengths: [size, 0, 0, 0],
            stack: 0,
            text: vec![0; size as usize],
            data: vec![],
            imports: vec![],
            relocations: offsets
                .iter()
                .map(|&offset| Relocation {
                    section: Section::Text,
                    offset,
                    encoding: Encoding::Low,
                    target: Reference::Section(Section::Text),
                    value: 0,
                    zero_extend: false,
                })
                .collect(),
            exports: vec![Export {
                name: "last".into(),
                segment: 2,
                value: 0x18000 + size - 1,
            }],
        };
        let d = inspect(&o65::wire::encode(&file).unwrap());
        assert_eq!(d["sections"][0]["base"], 0x18000);
        assert_eq!(d["sections"][0]["size"], size);
        assert_eq!(d["exports"][0]["value"], 0x18000 + size - 1);
        assert_eq!(
            d["relocations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["offset"].as_u64().unwrap() as u32)
                .collect::<Vec<_>>(),
            offsets
        );
    }
    let file = File {
        mode: o65::wire::MODE,
        bases: [0; 4],
        lengths: [3, 0, 0, 0],
        stack: 0,
        text: vec![0; 3],
        data: vec![],
        imports: (0..65537).map(|i| format!("host{i}")).collect(),
        relocations: vec![Relocation {
            section: Section::Text,
            offset: 0,
            encoding: Encoding::Long,
            target: Reference::Import(65536),
            value: 0,
            zero_extend: false,
        }],
        exports: vec![],
    };
    let d = inspect(&o65::wire::encode(&file).unwrap());
    assert_eq!(d["relocations"][0]["import"], 65536);
}

#[test]
fn writer_encodes_all_address_forms_and_carry_bytes() {
    use o65::{profile::*, wire::File};
    let text = vec![0xef, 0xbe, 0xef, 0xbe, 0x12, 0xef, 0xbe, 0x12];
    let relocs = [
        (0, Encoding::Low),
        (1, Encoding::High),
        (2, Encoding::Word),
        (4, Encoding::Bank),
        (5, Encoding::Long),
    ]
    .into_iter()
    .map(|(offset, encoding)| Relocation {
        section: Section::Text,
        offset,
        encoding,
        target: Reference::Section(Section::Data),
        value: 0x12beef,
        zero_extend: false,
    })
    .collect();
    let mut file = File {
        mode: o65::wire::MODE,
        bases: [0, 0x120000, 0, 0],
        lengths: [8, 0xbef0, 0, 0],
        stack: 0,
        text,
        data: vec![0; 0xbef0],
        imports: vec![],
        relocations: relocs,
        exports: vec![],
    };
    let d = inspect(&o65::wire::encode(&file).unwrap());
    assert_eq!(
        d["relocations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["value"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [0xef, 0xbeef, 0xbeef, 0x12beef, 0x12beef]
    );
    file.relocations.push(file.relocations[0].clone());
    assert!(o65::wire::encode(&file).is_err());
}

fn placement(bases: [u32; 3], fault: u32) -> o65::Placement {
    o65::Placement {
        bases,
        allowed: vec![o65::Region {
            address: 0x10000,
            size: 0xff0000,
        }],
        reserved: vec![],
        nmi_extra_stack: 0,
        providers: vec![o65::Provider {
            name: o65::profile::OVERFLOW.into(),
            address: fault,
            size: 2,
            contract: o65::profile::Contract::overflow(),
        }],
    }
}
fn loaded_bytes(image: &o65::RelocatedImage, address: u32, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| {
            let a = address + i as u32;
            let s = image
                .segments()
                .iter()
                .find(|s| a >= s.address && a < s.address + s.bytes.len() as u32)
                .unwrap();
            s.bytes[(a - s.address) as usize]
        })
        .collect()
}
#[test]
fn relocates_independently_specified_bytes_with_bank_carries() {
    let bytes = include_bytes!("../fixtures/o65/reference.o65");
    let decoded = o65::decode(include_bytes!("../fixtures/o65/vasm_native.o65")).unwrap();
    assert_eq!(decoded.relocations.len(), 6);
    for (bases, fault, expected) in [
        (
            [0x10000, 0x210000, 0x12fffc],
            0x48000,
            vec![0, 0, 0x13, 0, 3, 0, 0x13, 0, 0, 1],
        ),
        (
            [0x30000, 0x220100, 0x3410fc],
            0x58000,
            vec![0, 0x11, 0x34, 0, 3, 0x11, 0x34, 0, 0, 3],
        ),
    ] {
        let image = o65::relocate(bytes, &placement(bases, fault)).unwrap();
        assert_eq!(image.entry(), bases[0]);
        assert_eq!(image.stack_overflow(), fault);
        assert_eq!(loaded_bytes(&image, bases[1], 10), expected);
        assert_eq!(image.zero_fill()[0].address, bases[2]);
        assert_eq!(image.zero_fill()[0].size, 8);
        assert_eq!(image.segments().iter().filter(|s| s.executable).count(), 1);
    }
    let status = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/o65/build_reference.py"
        ))
        .arg("--check")
        .status()
        .unwrap();
    assert!(status.success());
}
#[test]
fn rejects_malformed_files_and_invalid_placements_without_mutating_input() {
    let bytes = include_bytes!("../fixtures/o65/reference.o65").to_vec();
    let original = bytes.clone();
    let base = placement([0x10000, 0x210000, 0x12fffc], 0x48000);
    for n in 0..bytes.len() {
        assert!(
            o65::relocate(&bytes[..n], &base).is_err(),
            "accepted prefix {n}"
        );
    }
    for (at, value) in [(0, 0), (6, 0), (7, 0xe2), (8, 1), (10, 0xff)] {
        let mut bad = bytes.clone();
        bad[at] = value;
        assert!(o65::relocate(&bad, &base).is_err());
    }
    // Descriptor begins at text offset 1, after the 45-byte o65 header.
    for at in [46, 50, 54, 56, 58, 59] {
        let mut bad = bytes.clone();
        bad[at] ^= 0xff;
        assert!(o65::relocate(&bad, &base).is_err());
    }
    let mut bad = bytes.clone();
    bad.push(0);
    assert!(o65::relocate(&bad, &base).is_err());
    for variant in 0..8 {
        let mut p = base.clone();
        match variant {
            0 => p.bases[0] += 4,
            1 => p.bases[1] = p.bases[0],
            2 => p.bases[2] = 0xfffffc,
            3 => p.providers.clear(),
            4 => p.providers[0].contract.abi = "wrong".into(),
            5 => p.reserved.push(o65::Region {
                address: 0x210000,
                size: 1,
            }),
            6 => p.allowed.clear(),
            _ => p.nmi_extra_stack = 1,
        }
        assert!(o65::relocate(&bytes, &p).is_err(), "variant {variant}");
    }
    assert_eq!(bytes, original);
    let mut rng = 1u64;
    for n in 0..512 {
        let mut bad = vec![0; n];
        for b in &mut bad {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            *b = rng as u8;
        }
        assert!(o65::decode(&bad).is_err());
    }
}
#[test]
fn compiler_artifact_loads_without_the_compiler_result() {
    for optimize in [false, true] {
        let bytes = {
            let p = mir(
                "CARD output,initial=[7] PROC Main() output=initial RETURN",
                optimize,
            );
            o65::write(&o65::prepare(&p, &Default::default()).unwrap()).unwrap()
        };
        let a = o65::relocate(&bytes, &placement([0x10000, 0x220000, 0x330000], 0x48000)).unwrap();
        let b = o65::relocate(&bytes, &placement([0x20000, 0x440004, 0x550004], 0x58000)).unwrap();
        assert_ne!(a.entry(), b.entry());
        assert_eq!(a.profile().routines[0].frame, b.profile().routines[0].frame);
        assert_eq!(a.task_headroom(), 26);
    }
}

#[test]
fn signed_addends_and_profile_exclusions_keep_checked_address_semantics() {
    use actionc::{
        mir65816::*,
        nir::SymbolId,
        target::{ByteOffset, ByteSize, TargetLayout},
    };
    let mut p = mir("PROC Other() RETURN PROC Main() Other() RETURN", false);
    let main = p.routines.iter().find(|r| r.entry.program).unwrap().id;
    p.data.push(Mir65816Data {
        id: Mir65816DataId::Static(SymbolId(900)),
        placement: Mir65816DataPlacement::Allocate,
        size: ByteSize::new(7),
        zero_fill: ByteSize::ZERO,
        mutable: false,
        ty: None,
        array: None,
        section: None,
        name: "addresses".into(),
        bytes: vec![0; 7],
        alignment: ByteSize::ONE,
        relocations: (0..4)
            .map(|byte| Mir65816Relocation {
                offset: ByteOffset::new(if byte == 3 { 3 } else { byte }),
                byte_index: if byte == 3 { None } else { Some(byte as u8) },
                width: ByteSize::new(if byte == 3 { 4 } else { 1 }),
                address_space: TargetLayout::CODE_ADDRESS_SPACE,
                target: Mir65816RelocationTarget::Code(main),
                addend: -1,
            })
            .collect(),
    });
    let artifact = o65::prepare(&p, &Default::default()).unwrap();
    let expected = 0x20000 + artifact.profile().entry - 1;
    let bytes = o65::write(&artifact).unwrap();
    let image = o65::relocate(&bytes, &placement([0x20000, 0, 0], 0x58000)).unwrap();
    let object = image
        .profile()
        .objects
        .iter()
        .find(|o| o.name == "addresses")
        .unwrap();
    let mut values = expected.to_le_bytes()[..3].to_vec();
    values.extend(expected.to_le_bytes());
    assert_eq!(
        loaded_bytes(&image, image.location(object.location), 7),
        values
    );
    p.data[0].relocations[0].target = Mir65816RelocationTarget::ImageEnd;
    assert!(
        o65::prepare(&p, &Default::default())
            .unwrap_err()
            .contains("ImageEnd")
    );
    p.data[0].relocations[0].target = Mir65816RelocationTarget::Code(main);
    p.data[0].relocations[0].byte_index = None;
    assert!(
        o65::prepare(&p, &Default::default())
            .unwrap_err()
            .contains("narrow")
    );
}
#[test]
fn corrupted_descriptors_and_late_streams_never_panic() {
    let bytes = include_bytes!("../fixtures/o65/reference.o65");
    let p = placement([0x10000, 0x210000, 0x12fffc], 0x48000);
    for at in 0..bytes.len() {
        let mut corrupt = bytes.to_vec();
        corrupt[at] ^= 0xff;
        assert!(
            std::panic::catch_unwind(|| o65::relocate(&corrupt, &p)).is_ok(),
            "panic at byte {at}"
        );
    }
    for range in [12..16, 16..20, 50..54] {
        let mut corrupt = bytes.to_vec();
        corrupt[range].fill(255);
        assert!(o65::relocate(&corrupt, &p).is_err());
    }
}
