mod support;
use support::*;

const VALUES: [i16; 11] = [-32768, -32767, -257, -256, -1, 0, 1, 255, 256, 32766, 32767];

fn source() -> String {
    let mut s = String::from(
        "INT a=$7100,b=$7102\nBYTE ARRAY out=$7200\n\
         BYTE FUNC Ret(INT x,y) RETURN(x<y)\n\
         PROC Save(BYTE value) out(24)=value RETURN\n\
         PROC Work(INT x,y) BYTE saved\n",
    );
    for (i, op) in ["<", "<=", ">", ">="].iter().enumerate() {
        s.push_str(&format!(
            "out({})=(x{op}y)\nIF x{op}y THEN out({})=1 ELSE out({})=0 FI\n",
            2 * i,
            8 + 2 * i,
            8 + 2 * i
        ));
    }
    s.push_str(
        "out(16)=(x<INT(255)) out(18)=(INT(255)<x)\n\
         saved=(x>=y) out(20)=Ret(x,y) out(22)=saved Save(saved)\n\
         IF saved THEN out(26)=1 ELSE out(26)=0 FI\n\
         IF saved THEN out(28)=1 ELSE out(28)=0 FI\n\
         x==+1 out(30)=(x>y) RETURN\nPROC Main() Work(a,b) RETURN\n",
    );
    s
}

fn pairs() -> Vec<(i16, i16)> {
    let mut pairs: Vec<_> = VALUES
        .into_iter()
        .flat_map(|a| VALUES.map(|b| (a, b)))
        .collect();
    let mut seed = 0x8162_2026u32;
    for _ in 0..64 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        pairs.push((seed as i16, (seed >> 16) as i16));
        pairs.push(((seed >> 16) as i16, seed as i16));
    }
    pairs
}

#[test]
fn signed_predicates_and_boolean_consumers_cover_runtime_boundary_pairs() {
    let s = source();
    for optimize in [false, true] {
        narrow_comparison::check_shape(&s, optimize, 2);
        let image = compile(&s, optimize);
        assert_eq!(
            image.to_json().unwrap(),
            compile(&s.replace('\n', "\r\n"), optimize)
                .to_json()
                .unwrap()
        );
        let caller = caller(image.entry);
        for (a, b) in pairs() {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller, mask);
                h.bus.ram[0x7100..0x7102].copy_from_slice(&a.to_le_bytes());
                h.bus.ram[0x7102..0x7104].copy_from_slice(&b.to_le_bytes());
                h.bus.ram[0x7200..0x7220].fill(0xa5);
                h.run();
                h.guards(mask);
                let relations = [a < b, a <= b, a > b, a >= b];
                let expected = relations.into_iter().chain(relations).chain([
                    a < 255,
                    255 < a,
                    a < b,
                    a >= b,
                    a >= b,
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
                if a == 32767 && b == -1 && mask == 0 {
                    narrow_comparison::record("signed-word-comparisons", optimize, &s, &image, &h);
                }
            }
        }
    }
}

#[test]
fn signed_size_probes_keep_frames_and_source_newlines_stable() {
    let source =
        include_str!("../../../docs/benchmarks/65816-signed-word-comparisons/planning-probes.act")
            .replace("\r\n", "\n");
    for optimize in [false, true] {
        let image = compile(&source, optimize);
        assert_eq!(
            image.to_json().unwrap(),
            compile(&source.replace('\n', "\r\n"), optimize)
                .to_json()
                .unwrap()
        );
        for r in &image.routines {
            if r.name.starts_with("Ret") || r.name.starts_with("Branch") {
                assert_eq!(r.fixed_frame, 6);
            }
        }
        let mut h = Harness::new(&image, &caller(image.entry), 0);
        h.run();
        h.guards(0);
        narrow_comparison::record("signed-size-probes", optimize, &source, &image, &h);
    }
}

#[test]
fn independent_a16_signed_subtraction_corrects_overflow_before_testing_sign() {
    use actionc_vm::native65816::{Inputs, Machine, Registers};
    for immediate in [false, true] {
        let code = assemble(
            &format!(
                "lda 2,s\nsec\n{}\nbvs overflow\njml corrected\noverflow: eor #$8000\ncorrected: bmi yes\nlda #0\nstp\nyes: lda #1\nstp\nnop",
                if immediate { "sbc #$ffff" } else { "sbc 4,s" }
            ),
            0x040000,
        );
        let n = usize::from(immediate);
        assert_eq!(&code[..3], &[0xa3, 2, 0x38]);
        assert_eq!(
            &code[3..5 + n],
            if immediate {
                &[0xe9, 0xff, 0xff][..]
            } else {
                &[0xe3, 4][..]
            }
        );
        assert_eq!(
            &code[5 + n..14 + n],
            &[0x70, 4, 0x5c, (14 + n) as u8, 0, 4, 0x49, 0, 0x80]
        );
        assert_eq!(code[14 + n], 0x30);
        for (a, b) in pairs() {
            let b = if immediate { -1 } else { b };
            for p in [0, 1, 0x40, 0x41, 4, 5, 0x44, 0x45] {
                let mut bus = Bus::new();
                bus.map(0x040000, &code, false);
                bus.map(0x4000, &[0xa5; 0x2000], true);
                bus.ram[0x5fe2..0x5fe4].copy_from_slice(&a.to_le_bytes());
                bus.ram[0x5fe4..0x5fe6].copy_from_slice(&b.to_le_bytes());
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
                assert_eq!(r.a, u16::from(a < b), "{a}/{b}/{p}");
                assert_eq!(
                    (r.x, r.y, r.s, r.d, r.dbr, r.p & 0x3c),
                    (0x1234, 0x5678, 0x5fe0, 0x2000, 0, p & 4)
                );
                assert!(bus.writes.is_empty());
            }
        }
    }
}
