mod support;
use actionc_vm::native65816::{Access, Inputs, Machine, Registers};
use std::{collections::BTreeSet, path::Path};
use support::*;

const EXPECT_FORWARDED: bool = true;

#[test]
fn frozen_corpus_sites_match_typed_operations_and_unchanged_baseline_images() {
    // Saved artifacts are optional outside the investigation workspace; the
    // portable generated-code probes below always run in CI.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let saved = root.join("target/single-word-edges-after/manifest.json");
    if !saved.exists() {
        return;
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(saved).unwrap()).unwrap();
    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/benchmarks/65816-local-accumulator-forwarding/expected-sites.json"
    ))
    .unwrap();
    for a in manifest["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["compiler"] == "actionc")
    {
        let command = a["commands"][0].as_array().unwrap();
        let at = command.iter().position(|v| v == "--layout").unwrap();
        let options =
            serde_json::from_slice(&std::fs::read(command[at + 1].as_str().unwrap()).unwrap())
                .unwrap();
        let p = actionc::compiler::native65816::prepare_file(
            command.last().unwrap().as_str().unwrap(),
            a["mode"] == "optimized",
            &Default::default(),
        )
        .unwrap();
        let c = p.compile(&options).unwrap();
        let index = forwarding::index(&p.mir, &c.machine, |id| {
            c.image
                .routines
                .iter()
                .find(|r| r.id == id.0)
                .unwrap()
                .address
        });
        let e = expected["builds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["case"] == a["case"] && e["mode"] == a["mode"])
            .unwrap();
        assert_eq!(
            index.len() + index.shared_return_sites,
            e["reload_pcs"].as_array().unwrap().len(),
            "{}/{}",
            a["case"],
            a["mode"]
        );
        assert!(index.values().all(|s| s.forwarded() == EXPECT_FORWARDED));
        if !EXPECT_FORWARDED {
            let old = actionc::mir65816::image::Image::from_json(
                &std::fs::read(a["image"].as_str().unwrap()).unwrap(),
            )
            .unwrap();
            assert_eq!(old.to_json().unwrap(), c.image.to_json().unwrap());
            assert_eq!(
                index.keys().map(|&pc| u64::from(pc)).collect::<Vec<_>>(),
                e["reload_pcs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap())
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn independent_ca65_reload_removal_preserves_full_value_flags_and_traffic() {
    for producer in ["lda 2,s", "lda 2,s\nclc\nadc #1", "lda 2,s\nsec\nsbc #1"] {
        let mut variants = vec![];
        for reload in [true, false] {
            let source = format!(
                "{producer}\nsta 4,s\n{}\n.export consume,done\nconsume: sta 6,s\ndone: stp\nnop",
                if reload { "lda 4,s" } else { "" }
            );
            variants.push(assemble_artifact(&source, 0x040000));
        }
        assert_eq!(variants[0].bytes.len(), variants[1].bytes.len() + 2);
        for value in [0u16, 1, 0xff, 0x100, 0x7fff, 0x8000, 0xffff] {
            for p in [0, 1, 0x40, 0xc5] {
                let mut observations = vec![];
                for a in &variants {
                    let mut bus = Bus::new();
                    bus.map(0x040000, &a.bytes, false);
                    bus.map(0x4000, &[0xa5; 0x2000], true);
                    bus.ram[0x5fe2..0x5fe4].copy_from_slice(&value.to_le_bytes());
                    let mut cpu = Machine::start_at(Registers {
                        a: 0xabcd,
                        x: 0x1234,
                        y: 0x5678,
                        s: 0x5fe0,
                        d: 0x2000,
                        dbr: 0,
                        pbr: 4,
                        pc: 0,
                        p,
                        emulation_mode: false,
                    });
                    assert!(
                        cpu.run_until(
                            &mut bus,
                            100,
                            |_| Inputs::default(),
                            |c| c.is_instruction_boundary() && c.pc() == a.symbols["consume"]
                        )
                        .unwrap()
                    );
                    let mut at = cpu.registers();
                    at.pc = 0;
                    assert!(
                        cpu.run_until(
                            &mut bus,
                            100,
                            |_| Inputs::default(),
                            |c| c.is_instruction_boundary() && c.pc() == a.symbols["done"]
                        )
                        .unwrap()
                    );
                    let mut end = cpu.registers();
                    end.pc = 0;
                    let reads: Vec<_> = bus
                        .reads
                        .iter()
                        .filter(|&&at| (0x4000..0x6000).contains(&at))
                        .copied()
                        .collect();
                    observations.push((
                        at,
                        end,
                        cpu.cycles(),
                        reads,
                        bus.writes.clone(),
                        bus.ram[0x4000..0x6000].to_vec(),
                    ));
                }
                let (a, b) = (&observations[0], &observations[1]);
                assert_eq!(a.0, b.0);
                assert_eq!(a.1, b.1);
                assert_eq!(a.2, b.2 + 5);
                assert_eq!(
                    a.3,
                    b.3.iter()
                        .copied()
                        .chain([0x5fe4, 0x5fe5])
                        .collect::<Vec<_>>()
                );
                assert_eq!(a.4, b.4);
                assert_eq!(a.5, b.5);
            }
        }
    }
}

const SOURCE: &str = include_str!("fixtures/accumulator_forwarding.act");
#[test]
fn generated_private_words_preserve_results_flags_stores_and_volatile_order() {
    let source = SOURCE.replace("\r\n", "\n");
    for optimize in [false, true] {
        let p = prepare(&source, optimize);
        let c = p.compile(&layout()).unwrap();
        let image =
            actionc::mir65816::image::Image::from_json(&c.image.to_json().unwrap()).unwrap();
        assert_eq!(
            image.to_json().unwrap(),
            compile(&source.replace('\n', "\r\n"), optimize)
                .to_json()
                .unwrap()
        );
        let sites = forwarding::index(&p.mir, &c.machine, |id| {
            image
                .routines
                .iter()
                .find(|r| r.id == id.0)
                .unwrap()
                .address
        });
        assert!(sites.values().all(|s| s.forwarded() == EXPECT_FORWARDED));
        assert_eq!(
            sites.values().map(|s| s.kind).collect::<BTreeSet<_>>(),
            BTreeSet::from([
                forwarding::Kind::Arithmetic,
                forwarding::Kind::Compare,
                forwarding::Kind::Store,
                forwarding::Kind::Return
            ])
        );
        let caller = caller(image.entry);
        let mut seen = BTreeSet::new();
        for (a, b) in [
            (0u16, 1u16),
            (1, 0),
            (0x7fff, 0x8000),
            (0x8000, 0x7fff),
            (0xffff, 0xffff),
        ] {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller, mask);
                h.bus.forwarded_words = sites.clone();
                for (name, v) in [("a", a), ("b", b)] {
                    let at = context::symbol(&image, name) as usize;
                    h.bus.ram[at..at + 2].copy_from_slice(&v.to_le_bytes());
                }
                h.bus.map(0xd000, &[0xff, 0x7f], true);
                h.bus.watched.extend([0xd000, 0xd001]);
                for _ in 0..100_000 {
                    if h.cpu.is_stopped() {
                        break;
                    }
                    if h.cpu.is_instruction_boundary() {
                        let pc = h.cpu.pc();
                        if let Some(s) = sites.get(&pc) {
                            assert!(s.valid(&h.bus));
                            seen.insert(pc);
                            let r = h.cpu.registers();
                            let value = h.bus.value(homes::address(r.s, r.d, s.slot), 2) as u16;
                            assert_eq!(r.a, value);
                            assert_eq!(
                                r.p & 0x82,
                                if value == 0 { 2 } else { 0 }
                                    | if value & 0x8000 != 0 { 0x80 } else { 0 }
                            );
                        }
                    }
                    h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                }
                assert!(h.cpu.is_stopped());
                h.guards(mask);
                for (name, v) in [
                    ("result", a.wrapping_sub(1)),
                    ("maximum", a.max(b)),
                    ("stored", a.wrapping_sub(1)),
                    ("before", 0x7fff),
                    ("after", 0x8000),
                ] {
                    assert_eq!(h.global(&image, name, 2), u32::from(v));
                }
                let actual: Vec<_> = h
                    .bus
                    .trace
                    .iter()
                    .map(|&(_, at, access)| (at, access))
                    .collect();
                assert_eq!(
                    actual,
                    vec![
                        (0xd000, Access::Read),
                        (0xd001, Access::Read),
                        (0xd000, Access::Write(0)),
                        (0xd001, Access::Write(0x80)),
                        (0xd000, Access::Read),
                        (0xd001, Access::Read)
                    ]
                );
            }
        }
        assert_eq!(seen, sites.keys().copied().collect());
        if let Ok(directory) = std::env::var("A816_QUALIFICATION_DIR") {
            let stem = Path::new(&directory).join(format!("accumulator-forwarding-{optimize}"));
            std::fs::write(stem.with_extension("act"), &source).unwrap();
            std::fs::write(stem.with_extension("a816.json"), image.to_json().unwrap()).unwrap();
            std::fs::write(stem.with_extension("metrics.json"), serde_json::to_vec_pretty(&serde_json::json!({"sites":sites.values().map(|s|serde_json::json!({"start":s.start,"producer":s.producer,"store":s.store,"slot":s.slot,"kind":format!("{:?}",s.kind)})).collect::<Vec<_>>(),"all_sites_reached":seen,"word_pairs":5,"incoming_masks":[0,4]})).unwrap()).unwrap();
        }
    }
}

#[test]
fn evidence_rejects_changed_bytes_and_separates_addressable_and_indirect_storage() {
    let source = "CARD result CARD FUNC Direct(CARD x) RETURN(x+1) CARD FUNC Indirect(CARD POINTER p) RETURN(p^) PROC Main() result=Direct(1) RETURN";
    for optimize in [false, true] {
        let p = prepare(source, optimize);
        let c = p.compile(&layout()).unwrap();
        let index = forwarding::index(&p.mir, &c.machine, |id| {
            c.image
                .routines
                .iter()
                .find(|r| r.id == id.0)
                .unwrap()
                .address
        });
        let mut bus = Bus::new();
        bus.load(&c.image);
        for site in index.values() {
            assert!(site.valid(&bus));
            for (offset, expected) in site.bytes.iter().enumerate() {
                if expected.is_none() {
                    continue;
                }
                let at = site.producer as usize + offset;
                bus.ram[at] ^= 0x80;
                assert!(!site.valid(&bus));
                bus.ram[at] ^= 0x80;
            }
            assert!(
                !c.image
                    .routines
                    .iter()
                    .find(|r| r.id == site.routine.0)
                    .unwrap()
                    .name
                    .contains("Indirect")
            );
        }
        assert!(!index.is_empty());
    }
}
