mod support;
use actionc_vm::native65816::Inputs;
use support::{context::routine, *};

const SOURCE: &str = "TYPE Parcel=[BYTE ARRAY prefix(82) BYTE ARRAY storage(32)]\n\
TYPE Cursor=[BYTE value]\n\
TYPE Shelf=[BYTE tag CARD ARRAY words(16)]\n\
TYPE Crate=[CARD serial Shelf content]\n\
Cursor POINTER FUNC Field(Parcel POINTER item) RETURN(Cursor POINTER(@item.storage))\n\
Cursor POINTER FUNC Chain(Parcel POINTER item) RETURN(Cursor POINTER(@item.storage(0)))\n\
Cursor POINTER FUNC Element(Shelf POINTER item) RETURN(Cursor POINTER(@item.words(3)))\n\
Cursor POINTER FUNC Nested(Crate POINTER item) RETURN(Cursor POINTER(@item.content.words(2)))\n\
Cursor POINTER FUNC Dynamic(Parcel POINTER item BYTE index) RETURN(Cursor POINTER(@item.storage(index)))\n\
Cursor POINTER FUNC Framed(Parcel POINTER item) BYTE ARRAY scratch(4) scratch(0)=7 RETURN(Cursor POINTER(@item.storage))\n\
PROC Main() RETURN\n";

fn caller(address: u32) -> String {
    format!(
        "tsc\nsec\nsbc #3\ntcs\nsep #$20\n.a8\nlda f:$007100\nsta 1,s\nlda f:$007101\nsta 2,s\nlda f:$007102\nsta 3,s\nrep #$20\n.a16\nlda #$d5aa\nldx #$beef\nldy #$cafe\njsl ${address:06x}\n.export returned\nreturned: sta f:$007200\ntxa\nsta f:$007202\ntsc\nclc\nadc #3\ntcs\nstp\nnop\n"
    )
}

fn reach(h: &mut Harness, pc: u32) {
    assert!(
        h.cpu
            .run_until(
                &mut h.bus,
                100_000,
                |_| Inputs::default(),
                |c| c.is_instruction_boundary() && c.pc() == pc
            )
            .unwrap()
    );
}

#[test]
fn returned_record_addresses_preserve_native_lanes_and_never_read_the_pointee() {
    for optimize in [false, true] {
        let prepared = prepare(SOURCE, optimize);
        for guarded in [false, true] {
            let mut options = layout();
            options.stack_checks = guarded;
            let compiled = prepared.compile(&options).unwrap();
            let image = &compiled.image;
            assert_eq!(
                image.to_json().unwrap(),
                prepare(&SOURCE.replace('\n', "\r\n"), optimize)
                    .compile(&options)
                    .unwrap()
                    .image
                    .to_json()
                    .unwrap()
            );
            // CARD fields are aligned to two bytes by the record layout.
            for (name, offset) in [
                ("Field", 82u32),
                ("Chain", 82),
                ("Element", 8),
                ("Nested", 8),
                ("Framed", 82),
            ] {
                let record = image.routines.iter().find(|r| r.name == name).unwrap();
                if name != "Framed" {
                    assert_eq!(
                        (record.size, record.fixed_frame, record.spill_bytes),
                        (21, 0, 0)
                    );
                    assert!(record.temporaries.is_empty());
                }
                if name == "Framed" {
                    assert!(record.fixed_frame >= 4);
                }
                let entry = routine(image, name);
                let assembled = assemble_artifact(&caller(entry), 0x040000);
                for input in [0u32, 1, 0x12ffaf, 0xffffae, 0xffffff, 0x800000, 0x12abcd] {
                    for mask in [0, 4] {
                        let mut h = Harness::new(image, &assembled.bytes, mask);
                        h.bus.ram[0x7100..0x7103].copy_from_slice(&input.to_le_bytes()[..3]);
                        h.bus.ram[0x7103] = 0xb7;
                        h.bus.ram[0x71ff] = 0x39;
                        h.bus.ram[0x7204] = 0x6e;
                        let expected = input.wrapping_add(offset) & 0xffffff;
                        h.bus.watched.insert(input);
                        h.bus.watched.insert(expected);
                        reach(&mut h, entry);
                        let incoming = u32::from(h.cpu.registers().s) + 4;
                        h.bus.reads.clear();
                        h.bus.writes.clear();
                        reach(&mut h, assembled.symbols["returned"]);
                        let r = h.cpu.registers();
                        assert_eq!(
                            u32::from(r.a) | (u32::from(r.x) << 16),
                            expected,
                            "{name}/{input:06x}/{optimize}/{guarded}/{mask}"
                        );
                        assert_eq!(r.p & 0x3c, mask);
                        assert_eq!(r.s, 0x5fed);
                        assert!(h.bus.reads.iter().all(|a| *a != incoming + 3));
                        assert!(
                            h.bus.trace.is_empty(),
                            "forming an address accessed its pointee"
                        );
                        h.run();
                        h.guards(mask);
                        assert_eq!(h.bus.value(0x7200, 4), expected);
                        assert_eq!(
                            &h.bus.ram[0x7100..0x7104],
                            &[input as u8, (input >> 8) as u8, (input >> 16) as u8, 0xb7]
                        );
                        assert_eq!((h.bus.ram[0x71ff], h.bus.ram[0x7204]), (0x39, 0x6e));
                    }
                }
            }
        }
    }
}
