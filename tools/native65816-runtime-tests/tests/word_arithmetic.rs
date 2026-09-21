mod support;
use actionc::mir65816::image::Image;
use actionc_vm::native65816::Access;
use support::*;

#[test]
fn native_stack_and_immediate_encodings_agree_with_ca65_and_execute() {
    use actionc_vm::native65816::{Inputs, Machine, Registers};
    let code = assemble(
        r#"
        rep #$20
        lda 2,s
        clc
        adc 4,s
        sta 6,s
        lda 2,s
        sec
        sbc 4,s
        sta 8,s
        lda #1
        sec
        sbc 2,s
        sta 10,s
        lda 2,s
        clc
        adc #255
        sta 12,s
        lda 2,s
        sec
        sbc #255
        sta 14,s
        stp
        nop
    "#,
        0x040000,
    );
    // Independent assembler checks the numeric encodings used by selection.
    assert_eq!(
        code,
        [
            0xc2, 0x20, 0xa3, 2, 0x18, 0x63, 4, 0x83, 6, 0xa3, 2, 0x38, 0xe3, 4, 0x83, 8, 0xa9, 1,
            0, 0x38, 0xe3, 2, 0x83, 10, 0xa3, 2, 0x18, 0x69, 255, 0, 0x83, 12, 0xa3, 2, 0x38, 0xe9,
            255, 0, 0x83, 14, 0xdb, 0xea,
        ]
    );
    for (a, b) in [(0u16, 1u16), (0xffff, 1), (0x8000, 0xffff), (0x100, 1)] {
        for p in [0, 1, 0x24, 0x25] {
            let mut bus = Bus::new();
            bus.map(0x040000, &code, false);
            bus.map(0x4000, &[0xa5; 0x2000], true);
            bus.ram[0x5fe2..0x5fe4].copy_from_slice(&a.to_le_bytes());
            bus.ram[0x5fe4..0x5fe6].copy_from_slice(&b.to_le_bytes());
            let registers = Registers {
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
            };
            let mut cpu = Machine::start_at(registers);
            assert!(
                cpu.run_until(&mut bus, 500, |_| Inputs::default(), |cpu| cpu.is_stopped())
                    .unwrap()
            );
            for (offset, expected) in [
                (6, a.wrapping_add(b)),
                (8, a.wrapping_sub(b)),
                (10, 1u16.wrapping_sub(a)),
                (12, a.wrapping_add(255)),
                (14, a.wrapping_sub(255)),
            ] {
                assert_eq!(bus.value(0x5fe0 + offset, 2), u32::from(expected));
            }
            let end = cpu.registers();
            assert_eq!(
                (end.s, end.d, end.dbr, end.x, end.y),
                (
                    registers.s,
                    registers.d,
                    registers.dbr,
                    registers.x,
                    registers.y
                )
            );
            assert_eq!(end.p & 0x3c, p & 4);
            assert!(
                bus.writes
                    .iter()
                    .all(|(address, _)| (0x5fe6..0x5ff0).contains(address))
            );
        }
    }
}

fn set(h: &mut Harness, image: &Image, name: &str, value: u32, bytes: usize) {
    let address = image.data.iter().find(|d| d.name == name).unwrap().address as usize;
    h.bus.ram[address..address + bytes].copy_from_slice(&value.to_le_bytes()[..bytes]);
}

#[test]
fn word_boundaries_operand_orders_and_carry_chains_match_host_arithmetic() {
    let values = [0u16, 1, 0xff, 0x100, 0x7fff, 0x8000, 0xffff];
    for ty in ["CARD", "INT"] {
        let source = format!(
            r#"
{ty} a,b,total,difference,plus,minus,leftminus,twice,chain,narrow,joined
BYTE byteinput,bytebefore,byteafter
PROC Work({ty} x,y)
  bytebefore=byteinput+1
  total=x+y difference=x-y
  plus=x+1 minus=x-1 leftminus=1-x twice=x+x
  chain=(x+y)-(x-y) narrow=x+BYTE(255)
  IF x=0 THEN joined=y+1 ELSE joined=y-1 FI
  byteafter=bytebefore-1
RETURN
PROC Main() Work(a,b) RETURN
"#
        );
        for optimize in [false, true] {
            let image = compile(&source, optimize);
            let caller = caller(image.entry);
            for a in values {
                for b in values {
                    for mask in [0, 4] {
                        let mut h = Harness::new(&image, &caller, mask);
                        set(&mut h, &image, "a", a.into(), 2);
                        set(&mut h, &image, "b", b.into(), 2);
                        set(&mut h, &image, "byteinput", 255, 1);
                        h.run();
                        h.guards(mask);
                        for (name, expected) in [
                            ("total", a.wrapping_add(b)),
                            ("difference", a.wrapping_sub(b)),
                            ("plus", a.wrapping_add(1)),
                            ("minus", a.wrapping_sub(1)),
                            ("leftminus", 1u16.wrapping_sub(a)),
                            ("twice", a.wrapping_add(a)),
                            ("chain", b.wrapping_add(b)),
                            ("narrow", a.wrapping_add(255)),
                            (
                                "joined",
                                if a == 0 {
                                    b.wrapping_add(1)
                                } else {
                                    b.wrapping_sub(1)
                                },
                            ),
                        ] {
                            assert_eq!(
                                h.global(&image, name, 2),
                                u32::from(expected),
                                "{ty}/{optimize}/{a:04x}/{b:04x}/{mask}/{name}"
                            );
                        }
                        assert_eq!(h.global(&image, "bytebefore", 1), 0);
                        assert_eq!(h.global(&image, "byteafter", 1), 255);
                    }
                }
            }
        }
    }
}

#[test]
fn arithmetic_keeps_volatile_byte_traces_and_reloads_aliased_words() {
    let source = r#"
VOLATILE CARD io=$D000
CARD before,after,observed
CARD POINTER p,q
PROC Change(CARD POINTER left,right CARD amount)
  left^=left^+amount
  right^=right^-1
  observed=left^
RETURN
PROC Main()
  before=io+1 io=before-2 after=io+1
  Change(p,q,1)
RETURN
"#;
    for optimize in [false, true] {
        let image = compile(source, optimize);
        let caller = caller(image.entry);
        for alias in [false, true] {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller, mask);
                h.bus.map(0xd000, &[0xff, 0xff], true);
                h.bus.watched.extend(0xd000..0xd002);
                let mut memory = [0xa5, 0xff, 0xff, 0x5d, 0xe3, 0xb7, 0, 0x80, 0x93];
                h.bus.map(0x12fffe, &memory, true);
                set(&mut h, &image, "p", 0x12ffff, 3);
                set(
                    &mut h,
                    &image,
                    "q",
                    if alias { 0x12ffff } else { 0x130004 },
                    3,
                );
                h.run();
                h.guards(mask);
                assert_eq!(h.global(&image, "before", 2), 0);
                assert_eq!(h.global(&image, "after", 2), 0xffff);
                assert_eq!(
                    h.global(&image, "observed", 2),
                    if alias { 0xffff } else { 0 }
                );
                if !alias {
                    memory[1..3].copy_from_slice(&[0, 0]);
                    memory[6..8].copy_from_slice(&[0xff, 0x7f]);
                }
                assert_eq!(&h.bus.ram[0x12fffe..0x130007], memory);
                let trace: Vec<_> = h
                    .bus
                    .trace
                    .iter()
                    .map(|(_, address, access)| (*address, *access))
                    .collect();
                assert_eq!(
                    trace,
                    [
                        (0xd000, Access::Read),
                        (0xd001, Access::Read),
                        (0xd000, Access::Write(0xfe)),
                        (0xd001, Access::Write(0xff)),
                        (0xd000, Access::Read),
                        (0xd001, Access::Read),
                    ]
                );
            }
        }
    }
}

#[test]
fn live_words_survive_direct_and_indirect_full_scratch_clobbers() {
    use actionc::{
        mir65816::{abi, image::AssemblyImport},
        nir::runtime_symbol_id,
    };
    let source = r#"
MODULE TEST
PUBLIC EXTERNAL CARD FUNC Smash(CARD ignored)
CARD input,result
CARD FUNC Work(CARD n)
  CARD FUNC POINTER cb(CARD x)
  cb=@Smash
RETURN(((n+1)-Smash(n))+(n-cb(n)))
PROC Main() result=Work(input) RETURN
ENDMODULE
"#;
    let smash = assemble(
        r#"
        sep #$20
        .a8
        ldx #63
        lda #$a7
    again:
        sta 0,x
        dex
        bpl again
        rep #$20
        .a16
        ldy #$dead
        ldx #$beef
        lda #$1234
        rtl
    "#,
        0x041000,
    );
    for optimize in [false, true] {
        let prepared = prepare(source, optimize);
        let symbol = runtime_symbol_id("TEST.Smash");
        let imported = prepared
            .mir
            .routines
            .iter()
            .find(|r| r.entry.external_symbol == Some(symbol))
            .unwrap();
        let mut options = layout();
        options.imports.push(AssemblyImport {
            symbol: symbol.0,
            signature: imported.signature.0,
            abi: abi::generated::ABI_NAME.into(),
            address: 0x041000,
            size: smash.len() as u32,
            stack_peak: 0,
            checks_stack: true,
            irq_effect: Default::default(),
        });
        let compiled = prepared.compile(&options).unwrap();
        let image = Image::from_json(&compiled.image.to_json().unwrap()).unwrap();
        let caller = caller(image.entry);
        let input = image
            .data
            .iter()
            .find(|d| d.name.to_uppercase().contains("_INPUT_"))
            .unwrap()
            .address as usize;
        for n in [0u16, 1, 0x7fff, 0x8000, 0xffff] {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller, mask);
                h.bus.ram[input..input + 2].copy_from_slice(&n.to_le_bytes());
                h.bus.map(0x041000, &smash, false);
                h.run();
                h.guards(mask);
                assert_eq!(
                    h.global(&image, "result", 2),
                    u32::from(
                        n.wrapping_add(1)
                            .wrapping_sub(0x1234)
                            .wrapping_add(n.wrapping_sub(0x1234))
                    )
                );
                assert!((0x2000..0x2040).all(|a| h.bus.writes.contains(&(a, 0xa7))));
            }
        }
    }
}
