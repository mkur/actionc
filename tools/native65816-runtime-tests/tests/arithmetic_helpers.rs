mod support;
use actionc::mir65816::{
    image::Image,
    o65::{self as format, profile::*},
};
use actionc_vm::native65816::{Inputs, Machine};
use support::*;
const FAULT: u32 = 0x049000;
fn image(source: &str, optimize: bool) -> Image {
    let mut options = layout();
    options.arithmetic_fault = Some(FAULT);
    let compiled = prepare(source, optimize).compile(&options).unwrap();
    Image::from_json(&compiled.image.to_json().unwrap()).unwrap()
}
fn address(i: &Image, name: &str) -> usize {
    i.data
        .iter()
        .find(|d| {
            d.name
                .to_ascii_uppercase()
                .contains(&format!("_{}_", name.to_ascii_uppercase()))
                || d.name.eq_ignore_ascii_case(name)
        })
        .unwrap()
        .address as usize
}
fn put(h: &mut Harness, at: usize, n: u32, width: usize) {
    h.bus.ram[at..at + width].copy_from_slice(&n.to_le_bytes()[..width]);
}
fn oracle(a: u32, b: u32, width: usize, signed: bool) -> (u32, u32) {
    let bits = width * 8;
    let mask = u32::MAX >> (32 - bits);
    if signed {
        let extend = |n: u32| (i64::from(n) << (64 - bits)) >> (64 - bits);
        let (a, b) = (extend(a), extend(b));
        ((a / b) as u32 & mask, (a % b) as u32 & mask)
    } else {
        (a / b, a % b)
    }
}
fn cases(width: usize) -> Vec<(u32, u32)> {
    let bits = width * 8;
    let mask = u32::MAX >> (32 - bits);
    let mut boundary = vec![
        0,
        1,
        2,
        3,
        7,
        8,
        15,
        16,
        127,
        128,
        255,
        256,
        257,
        32767,
        32768,
        65535,
        65536,
        0x7fffffff,
        0x80000000,
        mask - 1,
        mask,
    ];
    for bit in 0..bits {
        let v = 1u32 << bit;
        boundary.extend([v - 1, v, v.saturating_add(1)]);
    }
    for n in &mut boundary {
        *n &= mask;
    }
    boundary.sort_unstable();
    boundary.dedup();
    let mut result = vec![];
    for &a in &boundary {
        for &b in &boundary {
            result.push((a, b));
        }
    }
    let mut seed = 0x5eedcafeu32;
    for _ in 0..1024 {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let a = seed & mask;
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        result.push((a, seed & mask));
    }
    result
}
// Timing range of a compiled kernel over the independently checked input corpus.
// Includes the ordinary Main caller; this is an observed range, not a WCET proof.
fn record_range(
    kind: &str,
    ty: &str,
    optimize: bool,
    image: &Image,
    range: (u64, u64),
    maximum: (u32, u32),
) {
    if let Ok(dir) = std::env::var("A816_QUALIFICATION_DIR") {
        let facts = serde_json::json!({"operation":kind,"type":ty,"optimize":optimize,"minimum_cycles":range.0,"maximum_observed_cycles":range.1,"maximum_operands":[maximum.0,maximum.1],"code_bytes":image.routines.iter().map(|r|r.size).sum::<u32>(),"routines":image.routines.iter().map(|r|serde_json::json!({"name":r.name,"bytes":r.size,"frame":r.fixed_frame,"local_stack_peak":r.local_stack_peak})).collect::<Vec<_>>()});
        std::fs::write(
            std::path::Path::new(&dir)
                .join(format!("arithmetic-range-{kind}-{ty}-{optimize}.json")),
            serde_json::to_vec_pretty(&facts).unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn runtime_operands_match_independent_integer_oracle_in_raw_and_optimized_images() {
    for optimize in [false, true] {
        for (ty, width, signed) in [
            ("BYTE", 1, false),
            ("CARD", 2, false),
            ("INT", 2, true),
            ("SIZE", 3, false),
            ("LONGCARD", 4, false),
            ("LONGINT", 4, true),
        ] {
            let source = format!("{ty} a,b,q,r PROC Main() q=a/b r=a MOD b RETURN");
            let i = image(&source, optimize);
            let mut h = Harness::new(&i, &caller(i.entry), 0);
            h.bus.map(FAULT, &[0xdb, 0xea], false);
            let initial = h.cpu.registers();
            let mut range = (u64::MAX, 0);
            let mut maximum = (0, 0);
            let (aa, bb, qq, rr) = (
                address(&i, "a"),
                address(&i, "b"),
                address(&i, "q"),
                address(&i, "r"),
            );
            let inputs = if width == 1 {
                (0..256)
                    .flat_map(|a| (0..256).map(move |b| (a, b)))
                    .collect()
            } else {
                cases(width)
            };
            for (a, b) in inputs {
                h.cpu = Machine::start_at(initial);
                h.bus.writes.clear();
                h.bus.reads.clear();
                put(&mut h, aa, a, width);
                put(&mut h, bb, b, width);
                if b == 0 {
                    assert!(
                        h.cpu
                            .run_until(
                                &mut h.bus,
                                10000,
                                |_| Inputs::default(),
                                |c| c.is_instruction_boundary() && c.pc() == FAULT
                            )
                            .unwrap()
                    );
                    let r = h.cpu.registers();
                    assert_eq!((r.a, r.x, r.d, r.dbr, r.p & 0x3c), (1, r.s, 0x2000, 0, 0));
                    assert!(
                        !h.bus
                            .writes
                            .iter()
                            .any(|&(at, _)| (qq..qq + width).contains(&(at as usize))
                                || (rr..rr + width).contains(&(at as usize)))
                    );
                } else {
                    h.run();
                    h.guards(0);
                    range.0 = range.0.min(h.cpu.cycles());
                    if h.cpu.cycles() > range.1 {
                        range.1 = h.cpu.cycles();
                        maximum = (a, b);
                    }
                    let (q, r) = oracle(a, b, width, signed);
                    assert_eq!(
                        (h.bus.value(qq as u32, width), h.bus.value(rr as u32, width)),
                        (q, r),
                        "{ty} {a:x}/{b:x} optimize={optimize}"
                    );
                }
            }
            record_range("DIVMOD", ty, optimize, &i, range, maximum);
        }
    }
}
#[test]
fn multiplication_keeps_the_resolved_width_before_consumer_widening() {
    for optimize in [false, true] {
        for (ty, width, product_width) in [
            ("BYTE", 1, 2),
            ("CARD", 2, 2),
            ("INT", 2, 2),
            ("SIZE", 3, 2),
            ("LONGCARD", 4, 4),
            ("LONGINT", 4, 4),
        ] {
            let i = image(
                &format!("{ty} a,b LONGCARD result PROC Main() result=LONGCARD(a*b) RETURN"),
                optimize,
            );
            let mut h = Harness::new(&i, &caller(i.entry), 4);
            let initial = h.cpu.registers();
            let mut range = (u64::MAX, 0);
            let mut maximum = (0, 0);
            for (a, b) in cases(width) {
                h.cpu = Machine::start_at(initial);
                h.bus.writes.clear();
                h.bus.reads.clear();
                put(&mut h, address(&i, "a"), a, width);
                put(&mut h, address(&i, "b"), b, width);
                h.run();
                h.guards(4);
                range.0 = range.0.min(h.cpu.cycles());
                if h.cpu.cycles() > range.1 {
                    range.1 = h.cpu.cycles();
                    maximum = (a, b);
                }
                let low = a.wrapping_mul(b);
                // Narrow MUL has signed INT type; widening to LONGCARD sign-extends.
                let expected = if product_width == 2 {
                    i32::from(low as i16) as u32
                } else {
                    low
                };
                assert_eq!(h.global(&i, "result", 4), expected, "{ty} {a:x}*{b:x}");
            }
            record_range("MUL", ty, optimize, &i, range, maximum);
        }
    }
}
#[test]
fn independent_assembly_calls_each_helper_with_exact_arguments_and_result_lanes() {
    let mut metrics = vec![];
    for (ty, _width, signed) in [
        ("BYTE", 1, false),
        ("CARD", 2, false),
        ("INT", 2, true),
        ("SIZE", 3, false),
        ("LONGCARD", 4, false),
        ("LONGINT", 4, true),
    ] {
        let i = image(
            &format!("{ty} a,b,q,r,p PROC Main() q=a/b r=a MOD b p=a*b RETURN"),
            true,
        );
        for helper in i.routines.iter().filter(|r| r.name.starts_with("__a816_")) {
            let width = usize::from(helper.result_bytes);
            let offset = if width == 3 { 4 } else { width };
            let o = (offset + width) | 1;
            assert_eq!(helper.outgoing_bytes, o as u32);
            assert_eq!(helper.local_stack_peak, 0);
            let mut asm = format!("tsc\nsec\nsbc #{o}\ntcs\nsep #$20\n.a8\nlda #0\n");
            for n in 1..=o {
                asm += &format!("sta {n},s\n");
            }
            for (source, target) in [(0x7000, 1), (0x7004, offset + 1)] {
                for n in 0..width {
                    asm += &format!("lda f:${:06x}\nsta {},s\n", source + n, target + n);
                }
            }
            asm += &format!(
                "rep #$20\n.a16\njsl ${:06x}\nsta f:$007008\ntxa\nsta f:$00700a\ntsc\nclc\nadc #{o}\ntcs\nstp\nnop",
                helper.address
            );
            let code = assemble(&asm, 0x040000);
            for mask in [0, 4] {
                let mut h = Harness::new(&i, &code, mask);
                let a = if width == 1 {
                    0xf3
                } else if width == 3 {
                    0xabcdef
                } else {
                    0x8123abcd & (u32::MAX >> (32 - width * 8))
                };
                let b = if width == 1 { 13 } else { 0xfff1 };
                put(&mut h, 0x7000, a, width);
                put(&mut h, 0x7004, b, width);
                assert!(
                    h.cpu
                        .run_until(
                            &mut h.bus,
                            10000,
                            |_| Inputs::default(),
                            |c| c.is_instruction_boundary() && c.pc() == helper.address
                        )
                        .unwrap()
                );
                let begin = h.cpu.cycles();
                assert!(
                    h.cpu
                        .run_until(
                            &mut h.bus,
                            10000,
                            |_| Inputs::default(),
                            |c| c.is_instruction_boundary()
                                && !(helper.address..helper.address + helper.size)
                                    .contains(&c.pc())
                        )
                        .unwrap()
                );
                let body_cycles = h.cpu.cycles() - begin;
                h.run();
                h.guards(mask);
                let lowest_stack = h
                    .bus
                    .writes
                    .iter()
                    .map(|&(at, _)| at)
                    .filter(|at| (0x4000..0x6000).contains(at))
                    .min()
                    .unwrap();
                let observed_stack = 0x5ff1 - lowest_stack;
                assert_eq!(observed_stack, (o + 3) as u32);
                let scratch: std::collections::BTreeSet<_> = h
                    .bus
                    .writes
                    .iter()
                    .map(|&(at, _)| at)
                    .filter(|at| (0x2000..0x2040).contains(at))
                    .map(|at| at - 0x2000)
                    .collect();
                assert!(scratch.iter().all(|&offset| offset < 20));
                let expected = if helper.name.contains("mul") {
                    a.wrapping_mul(b) & (u32::MAX >> (32 - width * 8))
                } else {
                    let (q, r) = oracle(a, b, width, signed);
                    if helper.name.contains("div") { q } else { r }
                };
                assert_eq!(
                    h.bus.value(0x7008, if width <= 2 { 2 } else { 4 }),
                    expected,
                    "{}",
                    helper.name
                );
                metrics.push(serde_json::json!({"helper":helper.name,"bytes":helper.size,"cycles_with_ca65_caller":h.cpu.cycles(),"stack":o+3,"helper_frame":helper.fixed_frame,"scratch_offsets":scratch,"body_cycles_including_guard_and_rtl":body_cycles,"observed_stack":observed_stack,"left":a,"right":b,"irq_mask":mask}));
            }
        }
    }
    if let Ok(dir) = std::env::var("A816_QUALIFICATION_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join("arithmetic-metrics.json"),
            serde_json::to_vec_pretty(&metrics).unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn division_fault_and_nested_calls_survive_o65_serialization_and_relocation() {
    for optimize in [false, true] {
        let source = "LONGCARD a,b,q,r,live BYTE phase LONGCARD FUNC Work(LONGCARD n) RETURN(n/b) PROC Main() phase=1 live=a+LONGCARD(3) q=Work(a)+Work(a+LONGCARD(1)) r=a MOD b phase=2 RETURN";
        let bytes = o65::compile(source, optimize, vec![]);
        for variant in [0, 1] {
            let fault = if variant == 0 { FAULT } else { 0x069000 };
            let providers = vec![
                o65::fault(variant),
                format::Provider {
                    name: ARITHMETIC_FAULT.into(),
                    address: fault,
                    size: 2,
                    contract: Contract::arithmetic_fault(),
                },
            ];
            let placement = o65::placement(&bytes, variant, providers);
            let i = format::relocate(&bytes, &placement).unwrap();
            assert_eq!(i.arithmetic_fault(), Some(fault));
            let mut bad = placement.clone();
            bad.providers[1].contract = Contract::overflow();
            assert!(format::relocate(&bytes, &bad).is_err());
            let mut h = Harness::new_o65(&i, &caller(i.entry()), 0);
            h.bus.map(fault, &[0xdb, 0xea], false);
            let initial = h.cpu.registers();
            for b in [3, 0x80000001, 0] {
                h.cpu = Machine::start_at(initial);
                h.bus.writes.clear();
                put(&mut h, o65::object(&i, "a") as usize, 0xfffffffd, 4);
                put(&mut h, o65::object(&i, "b") as usize, b, 4);
                if b == 0 {
                    assert!(
                        h.cpu
                            .run_until(
                                &mut h.bus,
                                10000,
                                |_| Inputs::default(),
                                |c| c.pc() == fault && c.is_instruction_boundary()
                            )
                            .unwrap()
                    );
                    assert_eq!(h.bus.value(o65::object(&i, "phase"), 1), 1);
                } else {
                    h.run();
                    h.guards(0);
                    assert_eq!(
                        h.bus.value(o65::object(&i, "q"), 4),
                        (0xfffffffdu32 / b).wrapping_add(0xfffffffe / b)
                    );
                    assert_eq!(h.bus.value(o65::object(&i, "r"), 4), 0xfffffffd % b);
                    assert_eq!(h.bus.value(o65::object(&i, "live"), 4), 0);
                }
            }
        }
    }
}
#[test]
fn effectful_operands_compound_stores_and_banked_destinations_are_evaluated_once() {
    let source = r#"VOLATILE BYTE io=$7300
LONGCARD a,b,q,r,live
BYTE reads
LONGCARD FUNC Next() reads==+1 RETURN(a+LONGCARD(reads))
PROC Main()
  LONGCARD POINTER p
  live=a+LONGCARD(9)
  q=Next()/Next()
  r=LONGCARD(io)*LONGCARD(io)
  p=LONGCARD POINTER($12fffd)
  p^=a p^==/b
  q==+Next() MOD b
RETURN"#;
    for optimize in [false, true] {
        let i = image(source, optimize);
        let mut h = Harness::new(&i, &caller(i.entry), 0);
        h.bus.map(0x12fffd, &[0; 4], true);
        put(&mut h, address(&i, "a"), 100, 4);
        put(&mut h, address(&i, "b"), 7, 4);
        put(&mut h, 0x7300, 13, 1);
        h.run();
        h.guards(0);
        assert_eq!(h.global(&i, "reads", 1), 3);
        assert_eq!(h.global(&i, "q", 4), 5);
        assert_eq!(h.global(&i, "r", 4), 169);
        assert_eq!(h.global(&i, "live", 4), 109);
        assert_eq!(h.bus.value(0x12fffd, 4), 14);
        assert_eq!(h.bus.reads.iter().filter(|&&a| a == 0x7300).count(), 2);
    }
}
#[test]
fn constant_reductions_execute_all_powers_without_helpers() {
    for optimize in [false, true] {
        for (ty, width) in [("BYTE", 1), ("CARD", 2), ("SIZE", 3), ("LONGCARD", 4)] {
            let bits = width * 8;
            let mut source = format!(
                "{ty} a {ty} ARRAY q({bits}),r({bits}) LONGCARD ARRAY p({bits}) PROC Main()\n"
            );
            for bit in 0..bits {
                let k = if bit < 16 {
                    format!("{ty}(CARD({}))", 1u32 << bit)
                } else {
                    format!("{ty}({})", 1u32 << bit)
                };
                source += &format!("q({bit})=a/{k} r({bit})=a MOD {k} p({bit})=LONGCARD(a*{k})\n");
            }
            source += "RETURN";
            let i = image(&source, optimize);
            assert_eq!(i.version, 3);
            assert!(!i.routines.iter().any(|r| r.name.starts_with("__a816_")));
            let mut h = Harness::new(&i, &caller(i.entry), 4);
            let initial = h.cpu.registers();
            for a in [0, 1, 127, 128, 0x81234567, u32::MAX] {
                let a = a & (u32::MAX >> (32 - bits));
                h.cpu = Machine::start_at(initial);
                h.bus.writes.clear();
                h.bus.reads.clear();
                put(&mut h, address(&i, "a"), a, width);
                h.run();
                h.guards(4);
                for bit in 0..bits {
                    let product = a.wrapping_mul(1u32 << bit);
                    let product = if width < 4 {
                        i32::from(product as i16) as u32
                    } else {
                        product
                    };
                    assert_eq!(h.bus.value((address(&i, "p") + bit * 4) as u32, 4), product);
                    assert_eq!(
                        h.bus.value((address(&i, "q") + bit * width) as u32, width),
                        a / (1u32 << bit)
                    );
                    assert_eq!(
                        h.bus.value((address(&i, "r") + bit * width) as u32, width),
                        a % (1u32 << bit)
                    );
                }
            }
        }
    }
}
#[test]
fn runtime_zero_fault_preserves_prior_effects_and_never_stores_a_result() {
    for optimize in [false, true] {
        for mask in [0, 4] {
            let i = image(
                "CARD a,b,q BYTE before,after PROC Main() before=1 q=a/(b-b) after=1 RETURN",
                optimize,
            );
            let mut h = Harness::new(&i, &caller(i.entry), mask);
            h.bus.map(FAULT, &[0xdb, 0xea], false);
            assert!(
                h.cpu
                    .run_until(
                        &mut h.bus,
                        10000,
                        |_| Inputs::default(),
                        |c| c.pc() == FAULT && c.is_instruction_boundary()
                    )
                    .unwrap()
            );
            let r = h.cpu.registers();
            assert_eq!(
                (r.a, r.x, r.d, r.dbr, r.p & 0x3c),
                (1, r.s, 0x2000, 0, mask)
            );
            assert_eq!(h.global(&i, "before", 1), 1);
            assert_eq!(h.global(&i, "after", 1), 0);
            assert!(
                !h.bus
                    .writes
                    .iter()
                    .any(|&(a, _)| a == address(&i, "q") as u32)
            );
        }
    }
}
#[test]
fn zero_frame_helper_checks_floor_and_ceiling_before_any_scratch_or_argument_access() {
    let i = image("CARD a,b,q PROC Main() q=a/b RETURN", false);
    let helper = i
        .routines
        .iter()
        .find(|r| r.name == "__a816_div_u16_v1")
        .unwrap();
    for mask in [0, 4] {
        for s in [0u16, 0x4018, 0x5ff2] {
            let mut h = Harness::new(&i, &caller(i.entry), mask);
            let mut r = h.cpu.registers();
            r.s = s;
            r.pc = helper.address as u16;
            r.pbr = (helper.address >> 16) as u8;
            h.cpu = Machine::start_at(r);
            assert!(
                h.cpu
                    .run_until(
                        &mut h.bus,
                        1000,
                        |_| Inputs::default(),
                        |c| c.pc() == i.stack_overflow && c.is_instruction_boundary()
                    )
                    .unwrap()
            );
            let r = h.cpu.registers();
            assert_eq!((r.a, r.x, r.s, r.p & 0x3c), (0, s, s, mask));
            assert!(h.bus.writes.is_empty());
        }
        let mut h = Harness::new(&i, &caller(i.entry), mask);
        let mut r = h.cpu.registers();
        r.s = 0x401a;
        r.pc = helper.address as u16;
        r.pbr = (helper.address >> 16) as u8;
        h.cpu = Machine::start_at(r);
        h.bus.ram[0x2044..0x2046].copy_from_slice(&0x401au16.to_le_bytes());
        let body = i
            .segments
            .iter()
            .find(|s| s.address == helper.address)
            .unwrap()
            .bytes
            .windows(2)
            .position(|b| b == [0xa3, 4])
            .unwrap() as u32
            + helper.address;
        assert!(
            h.cpu
                .run_until(
                    &mut h.bus,
                    1000,
                    |_| Inputs::default(),
                    |c| c.is_instruction_boundary() && c.pc() == body
                )
                .unwrap()
        );
        assert!(h.bus.writes.is_empty());
    }
}

#[test]
fn explicit_terminal_fault_and_discarded_call_result_keep_the_fault_contract() {
    for optimize in [false, true] {
        let mut prepared = prepare("BYTE before,after PROC Main() before=1 RETURN", optimize);
        let main = prepared
            .mir
            .routines
            .iter_mut()
            .find(|r| r.entry.program)
            .unwrap();
        main.blocks.last_mut().unwrap().terminator =
            actionc::mir65816::Mir65816Terminator::ArithmeticFault;
        let mut options = layout();
        options.arithmetic_fault = Some(FAULT);
        let compiled = prepared.compile(&options).unwrap();
        let explicit = Image::from_json(&compiled.image.to_json().unwrap()).unwrap();
        let discarded = image(
            "CARD a,b BYTE before,after CARD FUNC Divide(CARD x,y) RETURN(x/y) PROC Main() before=1 Divide(a,b) after=1 RETURN",
            optimize,
        );
        for i in [explicit, discarded] {
            for mask in [0, 4] {
                let mut h = Harness::new(&i, &caller(i.entry), mask);
                h.bus.map(FAULT, &[0xdb, 0xea], false);
                assert!(
                    h.cpu
                        .run_until(
                            &mut h.bus,
                            10000,
                            |_| Inputs::default(),
                            |c| c.pc() == FAULT && c.is_instruction_boundary()
                        )
                        .unwrap()
                );
                let r = h.cpu.registers();
                assert_eq!((r.a, r.x, r.p & 0x3c), (1, r.s, mask));
                assert_eq!(h.global(&i, "before", 1), 1);
                assert_eq!(h.global(&i, "after", 1), 0);
            }
        }
    }
}

#[test]
fn emitted_caller_and_helpers_execute_in_different_program_banks() {
    for optimize in [false, true] {
        let p = prepare("CARD a,b,q,r PROC Main() q=a/b r=a MOD b RETURN", optimize);
        let machine = actionc::mir65816::emit::materialize(&p.mir).unwrap();
        let main = p.mir.routines.iter().find(|r| r.entry.program).unwrap().id;
        let size = machine
            .routines
            .iter()
            .find(|r| r.id == main)
            .unwrap()
            .code
            .bytes
            .len() as u32;
        let mut options = layout();
        options.code_origin = 0x01ffff - size;
        options.data_origin = 0x12fffc;
        options.arithmetic_fault = Some(FAULT);
        let i = p.compile(&options).unwrap().image;
        let i = Image::from_json(&i.to_json().unwrap()).unwrap();
        assert_eq!(i.entry >> 16, 1);
        assert!(
            i.routines
                .iter()
                .filter(|r| r.name.starts_with("__a816_"))
                .all(|r| r.address >> 16 == 2)
        );
        let mut h = Harness::new(&i, &caller(i.entry), 0);
        put(&mut h, address(&i, "a"), 65535, 2);
        put(&mut h, address(&i, "b"), 32768, 2);
        h.run();
        h.guards(0);
        assert_eq!(h.global(&i, "q", 2), 1);
        assert_eq!(h.global(&i, "r", 2), 32767);
    }
}
