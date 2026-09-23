//! Shared measurement of executed comparison probes; expectations live in tests.
use super::*;

pub fn record(name: &str, optimize: bool, source: &str, image: &Image, h: &Harness) {
    let facts = serde_json::json!({
        "code_bytes": image.routines.iter().map(|r|r.size).sum::<u32>(),
        "routines": image.routines.iter().map(|r|serde_json::json!({
            "name":r.name,"bytes":r.size,"frame":r.fixed_frame
        })).collect::<Vec<_>>(),
        "cycles":h.cpu.cycles(),
        "stack_reads":h.bus.reads.iter().filter(|&&a|(0x4000..0x6000).contains(&a)).count(),
        "stack_writes":h.bus.writes.iter().filter(|&&(a,_)|(0x4000..0x6000).contains(&a)).count(),
        "dp_reads":h.bus.reads.iter().filter(|&&a|(0x2000..0x2040).contains(&a)).count(),
        "dp_writes":h.bus.writes.iter().filter(|&&(a,_)|(0x2000..0x2040).contains(&a)).count(),
    });
    println!("{name}/{optimize}: {facts}");
    if let Ok(dir) = std::env::var("A816_QUALIFICATION_DIR") {
        let stem = Path::new(&dir).join(format!("{name}-{optimize}"));
        std::fs::write(stem.with_extension("act"), source).unwrap();
        std::fs::write(stem.with_extension("image.json"), image.to_json().unwrap()).unwrap();
        std::fs::write(
            stem.with_extension("metrics.json"),
            serde_json::to_vec_pretty(&facts).unwrap(),
        )
        .unwrap();
    }
}

pub fn check_shape(source: &str, optimize: bool, width: u32) {
    use actionc::mir65816::{Mir65816Op, Mir65816Terminator, Mir65816Value};
    let p = prepare(source, optimize);
    assert!(
        p.mir
            .routines
            .iter()
            .flat_map(|r| &r.blocks)
            .any(|b| matches!(
        (b.ops.last(), &b.terminator),
        (Some(Mir65816Op::Compare {dest, width:w, ..}),
         Mir65816Terminator::Branch {condition:Mir65816Value::Temp(id,_),..})
         if dest==id && w.get()==width))
    );
    assert!(
        p.mir
            .routines
            .iter()
            .flat_map(|r| &r.blocks)
            .any(|b| matches!(
        (b.ops.last(), &b.terminator),
        (Some(Mir65816Op::Compare {dest,width:w,..}),
         Mir65816Terminator::Return {value:Some(Mir65816Value::Temp(id,_)),..})
         if dest==id && w.get()==width))
    );
}

pub fn captured_values_survive_clobbers(pointer: bool) {
    use actionc::mir65816::{abi, image::AssemblyImport};
    use actionc_vm::native65816::Access;
    let ty = if pointer { "BYTE POINTER" } else { "BYTE" };
    let source = format!(
        "MODULE TEST\nPUBLIC EXTERNAL PROC Smash()\n\
        TYPE Box=[{ty} value]\nVOLATILE Box io=$D000\nBYTE ARRAY out=$7200\n\
        PROC Main() Box POINTER cell\n{ty} saved,observed\nBYTE flag\nPROC POINTER cb\n\
        cell=Box POINTER($12FFFF) cb=@Smash\n\
        saved=cell.value observed=io.value flag=(saved=observed)\n\
        Smash() out(0)=flag out(2)=(saved=observed) out(4)=(io.value=observed)\n\
        IF cell.value={ty}($34) THEN out(6)=1 ELSE out(6)=0 FI\n\
        cell.value={ty}($FF) cb() out(8)=(cell.value={ty}($34))\n\
        IF saved#observed THEN out(10)=1 ELSE out(10)=0 FI\nRETURN\nENDMODULE\n"
    );
    let smash = assemble(
        "sep #$20\n.a8\nlda #$34\nsta f:$12ffff\nlda #0\nsta f:$130000\nsta f:$130001\nldx #63\nlda #$a7\nagain: sta 0,x\ndex\nbpl again\nrep #$20\n.a16\nlda #$9876\nldx #$beef\nldy #$dead\nsep #$41\nrtl",
        0x041000,
    );
    for optimize in [false, true] {
        let p = prepare(&source, optimize);
        let symbol = actionc::nir::runtime_symbol_id("TEST.Smash");
        let signature = p
            .mir
            .routines
            .iter()
            .find(|r| r.entry.external_symbol == Some(symbol))
            .unwrap()
            .signature
            .0;
        let mut options = layout();
        options.imports.push(AssemblyImport {
            symbol: symbol.0,
            signature,
            abi: abi::generated::ABI_NAME.into(),
            address: 0x041000,
            size: smash.len() as u32,
            stack_peak: 0,
            checks_stack: true,
            irq_effect: Default::default(),
        });
        let image = p.compile(&options).unwrap().image;
        let caller = caller(image.entry);
        for value in [0u32, 0x10000, 0xffffff] {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller, mask);
                let width = if pointer { 3 } else { 1 };
                h.bus.map(0x041000, &smash, false);
                h.bus.map(0xd000, &value.to_le_bytes()[..width], true);
                h.bus.watched.extend(0xd000..0xd000 + width as u32);
                h.bus.map(
                    0x12fffe,
                    &[
                        0xa5,
                        value as u8,
                        (value >> 8) as u8,
                        (value >> 16) as u8,
                        0x5a,
                    ],
                    true,
                );
                h.bus.ram[0x7200..0x720c].fill(0xa5);
                h.run();
                h.guards(mask);
                for (i, result) in [1, 1, 1, 1, 1, 0].into_iter().enumerate() {
                    assert_eq!(h.bus.ram[0x7200 + 2 * i], result);
                    assert_eq!(h.bus.ram[0x7201 + 2 * i], 0xa5);
                }
                let expected: Vec<_> = (0..2)
                    .flat_map(|_| (0..width).map(|i| (0xd000 + i as u32, Access::Read)))
                    .collect();
                assert_eq!(
                    h.bus
                        .trace
                        .iter()
                        .map(|&(_, a, k)| (a, k))
                        .collect::<Vec<_>>(),
                    expected
                );
                assert_eq!(h.bus.ram[0x12fffe], 0xa5);
                assert_eq!(h.bus.ram[0x130002], 0x5a);
                assert!((0x2000..0x2040).all(|a| h.bus.writes.contains(&(a, 0xa7))));
            }
        }
    }
}
