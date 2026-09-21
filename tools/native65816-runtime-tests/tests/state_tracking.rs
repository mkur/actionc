mod support;
use actionc::mir65816::emit::proof::{self, Snapshot, Value, Width};
use actionc_vm::native65816::{Inputs, Machine, Registers};
use support::*;

fn known(value: Value, entry_s: u16) -> Option<(u16, u16)> {
    match value {
        Value::Constant(v, w) => Some((v, if w == Width::Byte { 0xff } else { 0xffff })),
        Value::StackAddress(offset) => Some((entry_s.wrapping_add(offset as u16), 0xffff)),
        _ => None,
    }
}
fn check(s: &Snapshot, r: Registers, entry_s: u16) {
    assert_eq!(
        (r.p & 0x20 != 0, r.p & 0x10 != 0),
        (s.m == Width::Byte, s.index == Width::Byte),
        "{s:?}"
    );
    assert_eq!(r.s, entry_s.wrapping_sub(s.depth as u16));
    for (fact, actual) in [(s.a, r.a), (s.x, r.x), (s.y, r.y)] {
        if let Some((v, mask)) = known(fact, entry_s) {
            assert_eq!(actual & mask, v, "{s:?}");
        }
    }
    for (left, right) in [(s.a, s.x), (s.a, s.y), (s.x, s.y)] {
        if let (Value::Opaque(a, w), Value::Opaque(b, _)) = (left, right) {
            if a == b {
                let values = [(s.a, r.a), (s.x, r.x), (s.y, r.y)];
                let got: Vec<_> = values
                    .iter()
                    .filter(|(v, _)| *v == left)
                    .map(|(_, v)| v & if w == Width::Byte { 0xff } else { 0xffff })
                    .collect();
                assert!(got.windows(2).all(|v| v[0] == v[1]));
            }
        }
    }
    if let Some((v, mask)) = known(s.nz, entry_s) {
        assert_eq!(r.p & 2 != 0, v == 0, "Z: {s:?}");
        assert_eq!(r.p & 0x80 != 0, v & ((mask >> 1) + 1) != 0, "N: {s:?}");
    }
    if let Some(c) = s.carry {
        assert_eq!(r.p & 1 != 0, c, "C: {s:?}");
    }
    if let Some(v) = s.overflow {
        assert_eq!(r.p & 0x40 != 0, v, "V: {s:?}");
    }
}
#[test]
fn typed_facade_encodings_and_claims_match_independent_ca65_and_vm() {
    for (left, right) in [
        (0, 0),
        (0xffff, 1),
        (0x7fff, 1),
        (0x8000, 0xffff),
        (0x100, 0xff),
    ] {
        let (code, trace) = proof::arithmetic_probe(left, right);
        let independent = assemble(
            &format!(
                "rep #$20\nlda #{left}\nclc\nadc #{right}\nsta 2,s\ntax\nsec\nsbc #{right}\ncmp #{left}\ntay\nsep #$20\n.a8\nlda #$80\nxba\nrep #$20\n.a16\ntsc\nclc\nadc #0\ntcs\nstp\nnop"
            ),
            0x40000,
        );
        assert_eq!(code.bytes, independent[..code.bytes.len()]);
        for irq in [0, 4] {
            let mut bus = Bus::new();
            bus.map(0x40000, &independent, false);
            bus.map(0x4000, &[0; 0x2000], true);
            let entry_s = 0x5fe0;
            let mut cpu = Machine::start_at(Registers {
                a: 0xabcd,
                x: 0x1234,
                y: 0x5678,
                s: entry_s,
                d: 0x2000,
                dbr: 0,
                pbr: 4,
                pc: 0,
                p: irq,
                emulation_mode: false,
            });
            for snapshot in &trace {
                assert!(
                    cpu.run_until(
                        &mut bus,
                        1000,
                        |_| Inputs::default(),
                        |c| c.is_instruction_boundary() && c.pc() == 0x40000 + snapshot.pc as u32
                    )
                    .unwrap()
                );
                check(snapshot, cpu.registers(), entry_s);
            }
            assert_eq!(
                bus.value(u32::from(entry_s) + 2, 2),
                u32::from(left.wrapping_add(right))
            );
        }
    }
}
