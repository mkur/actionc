mod support;
use actionc::mir65816::image::Image;
use actionc_vm::native65816::Inputs;
use support::{context::symbol, memory_runtime::*, *};

const SOURCE: &str = "MODULE TEST USE A816MEMORY
BYTE POINTER destination,source SIZE length BYTE value,operation
PROC Main()
 CASE operation OF
 WHEN 0 THEN
   A816MEMORY.Move(destination,source,length)
 WHEN 1 THEN
   A816MEMORY.Fill(destination,value,length)
 ELSE
   A816MEMORY.Clear(destination,length)
 ESAC
RETURN ENDMODULE";

fn set(h: &mut Harness, image: &Image, name: &str, value: u32, size: usize) {
    let at = symbol(image, name) as usize;
    h.bus.ram[at..at + size].copy_from_slice(&value.to_le_bytes()[..size]);
}

#[test]
fn exact_extents_overlap_odd_tails_and_24_bit_counts() {
    for optimize in [false, true] {
        let (image, assembly) = compile_memory(SOURCE, optimize);
        assert!(assembly.bytes.len() < 512);
        for count in [0u32, 1, 2, 3, 39, 40, 959, 960, 65535, 65536, 65537] {
            for (operation, destination, source) in [
                (0, 0x12ffefu32, 0x12fff0u32),
                (0, 0x12fff1, 0x12fff0),
                (0, 0x12fff0, 0x12fff0),
                (0, 0x140001, 0x12fff0),
                (1, 0x12ffff, 0),
                (2, 0x12ffff, 0),
            ] {
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller(image.entry), mask);
                    h.bus.map(ORIGIN, &assembly.bytes, false);
                    // One guarded ordinary-memory extent includes both source
                    // and target. Check every byte, including the odd neighbors.
                    let base = 0x12ff00;
                    let initial: Vec<u8> = (0..0x20210).map(|n| (n * 37 + 11) as u8).collect();
                    h.bus.map(base, &initial, true);
                    let mut expected = initial;
                    let dst = (destination - base) as usize;
                    if operation == 0 {
                        let src = (source - base) as usize;
                        expected.copy_within(src..src + count as usize, dst);
                    } else {
                        expected[dst..dst + count as usize].fill(if operation == 1 {
                            0xc7
                        } else {
                            0
                        });
                    }
                    set(&mut h, &image, "destination", destination, 3);
                    set(&mut h, &image, "source", source, 3);
                    set(&mut h, &image, "length", count, 3);
                    set(&mut h, &image, "value", 0xc7, 1);
                    set(&mut h, &image, "operation", operation, 1);
                    h.run();
                    h.guards(mask);
                    assert_eq!(
                        &h.bus.ram[base as usize..base as usize + expected.len()],
                        expected,
                        "{optimize}/{mask}/{operation}/{count}"
                    );
                }
            }
        }
    }
}

#[test]
fn irq_and_nmi_reenter_fill_without_corrupting_an_interrupted_move() {
    for optimize in [false, true] {
        let (image, memory) = compile_memory(SOURCE, optimize);
        let handler = assemble(
            &format!(
                ".smart\nrep #$30\npha\nphx\nphy\nphd\nphb\n\
            lda #$2300\ntcd\ntsc\nsec\nsbc #7\ntcs\n\
            lda #$0100\nsta 1,s\nsep #$20\nlda #$30\nsta 3,s\n\
            lda #$c7\nsta 4,s\nlda #0\nsta 7,s\nrep #$20\n\
            lda #17\nsta 5,s\njsl ${:06x}\ntsc\nclc\nadc #7\ntcs\n\
            inc a:$7820\nplb\npld\nply\nplx\npla\nrti",
                memory.symbols["a816_memory_fill"]
            ),
            0x9000,
        );
        for nmi in [false, true] {
            let mut h = Harness::new(&image, &caller(image.entry), 0);
            h.bus.map(ORIGIN, &memory.bytes, false);
            h.bus.map(0x9000, &handler, false);
            h.bus
                .map(if nmi { 0xffea } else { 0xffee }, &[0, 0x90], false);
            h.bus.map(0x2300, &[0xa5; 256], true);
            h.bus.map(0x7820, &[0, 0], true);
            h.bus.map(0x3000ff, &[0xa5; 19], true);
            let initial: Vec<u8> = (0..2048).map(|n| (n * 13) as u8).collect();
            h.bus.map(0x12ff00, &initial, true);
            let mut expected = initial;
            expected.copy_within(0..960, 1);
            set(&mut h, &image, "destination", 0x12ff01, 3);
            set(&mut h, &image, "source", 0x12ff00, 3);
            set(&mut h, &image, "length", 960, 3);
            let mut injected = 0;
            let mut boundaries = 0;
            let mut pending = false;
            for _ in 0..200_000 {
                if h.cpu.is_stopped() {
                    break;
                }
                let active = h.cpu.is_instruction_boundary()
                    && h.cpu.registers().d == 0x2000
                    && (ORIGIN..ORIGIN + memory.bytes.len() as u32).contains(&h.cpu.pc());
                if active {
                    boundaries += 1;
                }
                let pulse = active && boundaries % 31 == 0;
                injected += usize::from(pulse);
                pending |= pulse;
                let before = h.bus.value(0x7820, 2);
                h.cpu
                    .tick(
                        &mut h.bus,
                        Inputs {
                            nmi: nmi && pulse,
                            irq: !nmi && pending,
                            ..Default::default()
                        },
                    )
                    .unwrap();
                if h.bus.value(0x7820, 2) != before {
                    pending = false;
                }
            }
            assert!(h.cpu.is_stopped());
            h.guards(0);
            assert!(injected > 10 && h.bus.value(0x7820, 2) > 10);
            assert_eq!(&h.bus.ram[0x12ff00..0x130700], expected);
            assert_eq!(h.bus.ram[0x3000ff], 0xa5);
            assert_eq!(&h.bus.ram[0x300100..0x300111], &[0xc7; 17]);
            assert_eq!(h.bus.ram[0x300111], 0xa5);
        }
    }
}
