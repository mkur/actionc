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
