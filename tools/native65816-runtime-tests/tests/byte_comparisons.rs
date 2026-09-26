mod support;
use support::*;

#[test]
fn byte_captures_and_boolean_results_survive_alias_and_call_clobbers() {
    narrow_comparison::captured_values_survive_clobbers(false);
}

fn source() -> String {
    let mut s = String::from(
        "BYTE a=$7100,b=$7101\nBYTE ARRAY out=$7200\n\
        BYTE FUNC Ret(BYTE x,y) RETURN(x<y)\n\
        PROC Save(BYTE value) out(32)=value RETURN\n\
        PROC Work(BYTE x,y) BYTE saved\n",
    );
    for (i, op) in ["=", "#", "<", "<=", ">", ">="].iter().enumerate() {
        s.push_str(&format!(
            "out({})=(x{op}y)\nIF x{op}y THEN out({})=1 ELSE out({})=0 FI\n",
            2 * i,
            12 + 2 * i,
            12 + 2 * i
        ));
    }
    s.push_str(
        "out(24)=(x<BYTE(128)) out(26)=(BYTE(128)<x)\n\
        saved=(x>=y) out(28)=Ret(x,y) out(30)=saved Save(saved)\n\
        x==+1 out(34)=(x>y) RETURN\nPROC Main() Work(a,b) RETURN\n",
    );
    s
}

#[test]
fn byte_predicates_and_boolean_consumers_cover_runtime_boundary_pairs() {
    let s = source();
    for optimize in [false, true] {
        narrow_comparison::check_shape(&s, optimize, 1);
        let image = compile(&s, optimize);
        assert!(
            image
                .routines
                .iter()
                .find(|r| r.name == "Ret")
                .unwrap()
                .size
                <= 120
        );
        assert!(
            image.routines.iter().map(|r| r.size).sum::<u32>() < if optimize { 2986 } else { 2998 }
        );
        assert_eq!(
            image.to_json().unwrap(),
            compile(&s.replace('\n', "\r\n"), optimize)
                .to_json()
                .unwrap()
        );
        let caller = caller(image.entry);
        for a in [0u8, 1, 0x7f, 0x80, 0xfe, 0xff] {
            for b in [0u8, 1, 0x7f, 0x80, 0xfe, 0xff] {
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller, mask);
                    h.bus.ram[0x7100..0x7102].copy_from_slice(&[a, b]);
                    h.bus.ram[0x7200..0x7224].fill(0xa5);
                    h.run();
                    h.guards(mask);
                    let relations = [a == b, a != b, a < b, a <= b, a > b, a >= b];
                    let expected = relations.into_iter().chain(relations).chain([
                        a < 128,
                        128 < a,
                        a < b,
                        a >= b,
                        a >= b,
                        a.wrapping_add(1) > b,
                    ]);
                    for (i, truth) in expected.enumerate() {
                        assert_eq!(
                            h.bus.ram[0x7200 + 2 * i],
                            u8::from(truth),
                            "{optimize}/{a}/{b}/{mask}/{i}"
                        );
                        assert_eq!(h.bus.ram[0x7201 + 2 * i], 0xa5);
                    }
                    if a == 0x80 && b == 0x7f && mask == 0 {
                        narrow_comparison::record("byte-comparisons", optimize, &s, &image, &h);
                    }
                }
            }
        }
    }
}

#[test]
fn independent_a8_cmp_preserves_hidden_b_and_flags_through_a16_restoration() {
    use actionc_vm::native65816::{Inputs, Machine, Registers};
    for immediate in [false, true] {
        let code = assemble(
            &format!(
                "sep #$20\n.a8\nlda 2,s\n{}\nrep #$20\n.a16\nstp\nnop",
                if immediate { "cmp #$80" } else { "cmp 4,s" }
            ),
            0x040000,
        );
        assert_eq!(
            code,
            vec![
                0xe2,
                0x20,
                0xa3,
                2,
                if immediate { 0xc9 } else { 0xc3 },
                if immediate { 0x80 } else { 4 },
                0xc2,
                0x20,
                0xdb,
                0xea
            ]
        );
        for a in [0u8, 1, 0x7f, 0x80, 0xff] {
            for b in [0u8, 1, 0x7f, 0x80, 0xff] {
                let b = if immediate { 0x80 } else { b };
                for p in [0, 1, 0x40, 0x41] {
                    let mut bus = Bus::new();
                    bus.map(0x040000, &code, false);
                    bus.map(0x4000, &[0xa5; 0x2000], true);
                    bus.ram[0x5fe2] = a;
                    bus.ram[0x5fe4] = b;
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
                        cpu.run_until(&mut bus, 100, |_| Inputs::default(), |c| c.is_stopped())
                            .unwrap()
                    );
                    let r = cpu.registers();
                    assert_eq!(r.a, 0xab00 | u16::from(a));
                    assert_eq!(
                        r.p & 0xc3,
                        (p & 0x40)
                            | (a.wrapping_sub(b) & 0x80)
                            | u8::from(a >= b)
                            | (u8::from(a == b) << 1)
                    );
                    assert_eq!(r.p & 0x30, 0);
                    assert_eq!(r.x, 0x1234);
                    assert_eq!(r.y, 0x5678);
                }
            }
        }
    }
}

#[test]
fn byte_and_word_zero_tests_preserve_branches_and_materialized_results() {
    use actionc::mir65816::o65 as format;
    for (ty, width) in [("BYTE", 1usize), ("CARD", 2), ("INT", 2)] {
        let source = format!(
            "{ty} input=$7100\nBYTE ARRAY out=$7200\nBYTE FUNC Equal({ty} x) RETURN(x=0)\nBYTE FUNC Different({ty} x) RETURN(0#x)\nPROC Work({ty} x)\nout(0)=Equal(x) out(2)=Different(x)\nIF x=0 THEN out(4)=17 ELSE out(4)=31 FI\nIF 0#x THEN out(6)=17 ELSE out(6)=31 FI\nout(8)=(x<0) out(10)=(0<x) RETURN\nPROC Main() Work(input) RETURN\n"
        );
        for optimize in [false, true] {
            let image = compile(&source, optimize);
            assert_eq!(
                image.to_json().unwrap(),
                compile(&source.replace('\n', "\r\n"), optimize)
                    .to_json()
                    .unwrap()
            );
            let object = o65::compile(&source, optimize, vec![]);
            for variant in 0..3 {
                let loaded = (variant > 0).then(|| {
                    format::relocate(
                        &object,
                        &o65::placement(&object, variant - 1, vec![o65::fault(variant - 1)]),
                    )
                    .unwrap()
                });
                for value in [0u16, 1, 0x7f, 0x80, 0xff, 0x100, 0x7fff, 0x8000, 0xffff] {
                    let value = if width == 1 { value & 255 } else { value };
                    let signed = if ty == "INT" {
                        i32::from(value as i16)
                    } else {
                        i32::from(value)
                    };
                    let mut h = if let Some(l) = &loaded {
                        Harness::new_o65(l, &caller(l.entry()), 0)
                    } else {
                        Harness::new(&image, &caller(image.entry), 0)
                    };
                    h.bus.ram[0x7100..0x7102].copy_from_slice(&value.to_le_bytes());
                    h.bus.ram[0x7200..0x720c].fill(0xa5);
                    h.run();
                    h.guards(0);
                    for (i, want) in [
                        u8::from(value == 0),
                        u8::from(value != 0),
                        if value == 0 { 17 } else { 31 },
                        if value != 0 { 17 } else { 31 },
                        u8::from(signed < 0),
                        u8::from(signed > 0),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        assert_eq!(
                            h.bus.ram[0x7200 + 2 * i],
                            want,
                            "{ty}/{value}/{optimize}/{variant}/{i}"
                        );
                        assert_eq!(h.bus.ram[0x7201 + 2 * i], 0xa5);
                    }
                }
            }
        }
    }
}
