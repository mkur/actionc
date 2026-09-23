mod support;
use actionc_vm::native65816::{Inputs, Machine, Registers};
use support::*;

const SOURCE: &str = "CARD input,result CARD FUNC Work(CARD a) RETURN(a+1) PROC Empty() RETURN PROC Main() CARD FUNC POINTER cb(CARD a) cb=@Work result=Work(input)+cb(input) Empty() RETURN";

fn reference(amount: u16, origin: u32, fault: u32, short: bool) -> Vec<u8> {
    let branch = |op: &str, inverse: &str, target: &str| {
        if short {
            format!("{op} {target}\n")
        } else {
            format!("{inverse} :+\njml {target}\n:\n")
        }
    };
    assemble(
        &format!(
            "tsc\ntax\ncmp $46\n{}{}jml fault\nwithin: sec\nsbc #{amount}\n{}cmp $44\n{}fault: lda #{amount}\njml ${fault:06x}\ndone: nop\n",
            branch("bcc", "bcs", "within"),
            branch("beq", "bne", "within"),
            branch("bcc", "bcs", "fault"),
            branch("bcs", "bcc", "done")
        ),
        origin,
    )
}

fn run(
    code: &[u8],
    start: u32,
    done: u32,
    fault: u32,
    s: u16,
    floor: u16,
    ceiling: u16,
    p: u8,
) -> (Registers, Vec<u32>, u64, bool) {
    let mut bus = Bus::new();
    bus.map(start, code, false);
    bus.map(0x2000, &[0; 256], true);
    bus.map(fault, &[0xdb, 0xea], false);
    bus.ram[0x2044..0x2046].copy_from_slice(&floor.to_le_bytes());
    bus.ram[0x2046..0x2048].copy_from_slice(&ceiling.to_le_bytes());
    let mut cpu = Machine::start_at(Registers {
        a: 0xabcd,
        x: 0x1234,
        y: 0x5678,
        s,
        d: 0x2000,
        dbr: 0,
        pbr: (start >> 16) as u8,
        pc: start as u16,
        p,
        emulation_mode: false,
    });
    assert!(
        cpu.run_until(
            &mut bus,
            100,
            |_| Inputs::default(),
            |c| c.is_instruction_boundary() && [done, fault].contains(&c.pc())
        )
        .unwrap()
    );
    assert!(bus.writes.is_empty());
    let failed = cpu.pc() == fault;
    let mut r = cpu.registers();
    r.pc = 0;
    r.pbr = 0; // compare corresponding logical exits, not relocated PCs
    (
        r,
        bus.reads
            .into_iter()
            .filter(|a| (0x2000..0x2100).contains(a))
            .collect(),
        cpu.cycles(),
        failed,
    )
}

#[test]
fn emitted_entry_and_call_guards_match_independent_reference_on_all_boundary_paths() {
    let mut records = vec![];
    for optimize in [false, true] {
        let compiled = prepare(SOURCE, optimize).compile(&layout()).unwrap();
        let sites = guard::sites(&compiled);
        assert!(sites.iter().any(|s| s.amount == 0));
        assert!(sites.iter().any(|s| s.amount == 6)); // word call: O=3 + JSL=3
        assert!(sites.iter().any(|s| s.amount == 9)); // word indirect: O=3 + transfer=6
        for site in sites {
            let segment = compiled
                .image
                .segments
                .iter()
                .find(|s| s.address <= site.start && site.end <= s.address + s.bytes.len() as u32)
                .unwrap();
            let bytes = &segment.bytes
                [(site.start - segment.address) as usize..(site.end - segment.address) as usize];
            let fault = compiled.image.stack_overflow;
            let old = reference(site.amount, site.start, fault, false);
            let short = reference(site.amount, site.start, fault, true);
            assert_eq!(old.len() - short.len(), 16);
            let expected = if bytes.len() == 45 { &old } else { &short };
            assert_eq!(bytes, &expected[..expected.len() - 1]);
            let candidate = 0x5f00 - site.amount;
            let mut cases = vec![
                (0x5f00, candidate, 0x5f01),
                (0x5f00, candidate, 0x5f00),
                (0x5f00, candidate, 0x5eff),
                (0x5f00, candidate + 1, 0x5f01),
                (0x5f00, candidate - 1, 0x5f01),
                (0x8000, 1, 0xffff),
                (0xffff, 0, 0xffff),
            ];
            if site.amount > 0 {
                cases.push((site.amount - 1, 0, 0xffff));
            } else {
                cases.push((0, 0, 0));
            }
            for (s, floor, ceiling) in cases {
                for p in (0..=255u8).filter(|p| p & 0x38 == 0) {
                    let actual = run(bytes, site.start, site.end, fault, s, floor, ceiling, p);
                    let before = run(
                        &old,
                        site.start,
                        site.start + 45,
                        fault,
                        s,
                        floor,
                        ceiling,
                        p,
                    );
                    let compact = run(
                        &short,
                        site.start,
                        site.start + 29,
                        fault,
                        s,
                        floor,
                        ceiling,
                        p,
                    );
                    assert_eq!(
                        (&actual.0, &actual.1, actual.3),
                        (&before.0, &before.1, before.3)
                    );
                    assert_eq!(
                        (&compact.0, &compact.1, compact.3),
                        (&before.0, &before.1, before.3)
                    );
                    assert!(compact.2 < before.2);
                    let failed =
                        s > ceiling || s.checked_sub(site.amount).is_none_or(|n| n < floor);
                    assert_eq!(actual.3, failed);
                    assert_eq!(
                        (actual.0.a, actual.0.x, actual.0.y, actual.0.s),
                        (
                            if failed { site.amount } else { s - site.amount },
                            s,
                            0x5678,
                            s
                        )
                    );
                    if p == 0 || p == 4 {
                        records.push(serde_json::json!({"optimize":optimize,"routine":site.routine,
                            "amount":site.amount,"bytes":bytes.len(),"s":s,"floor":floor,"ceiling":ceiling,
                            "p":p,"failed":failed,"cycles":actual.2,"legacy_cycles":before.2,
                            "compact_cycles":compact.2,"exit_p":actual.0.p,"reads":actual.1}));
                    }
                }
            }
        }
    }
    if let Ok(dir) = std::env::var("A816_QUALIFICATION_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join("guard-observations.json"),
            serde_json::to_vec_pretty(&records).unwrap(),
        )
        .unwrap();
    }
}
