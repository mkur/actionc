mod support;
use support::{context::symbol, *};

#[test]
fn unlink_preserves_banked_links_and_touches_only_the_two_neighbor_fields() {
    let source = "TYPE Node=[Node POINTER ln_Succ Node POINTER ln_Pred] \
        Node POINTER selected \
        PROC Remove(Node POINTER item) Node POINTER previous,following \
        previous=item.ln_Pred following=item.ln_Succ \
        previous.ln_Succ=following following.ln_Pred=previous RETURN \
        PROC Main() Remove(selected) RETURN";
    for optimize in [false, true] {
        let image = compile(source, optimize);
        // Rotate a crossing in the low word, the bank byte, and the +3 field
        // address through all three nodes. None of their bank bytes is zero.
        for nodes in [
            [0x21ffffu32, 0x32fffc, 0x43fffe],
            [0x32fffc, 0x43fffe, 0x21ffff],
            [0x43fffe, 0x21ffff, 0x32fffc],
        ] {
            let [previous, item, following] = nodes;
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller(image.entry), mask);
                let selected = symbol(&image, "selected") as usize;
                h.bus.ram[selected..selected + 3].copy_from_slice(&item.to_le_bytes()[..3]);
                let mut expected = Vec::new();
                for (node, succ, pred) in [
                    (previous, item, 0xabcdefu32),
                    (item, following, previous),
                    (following, 0xfedcba, item),
                ] {
                    let mut bytes = [0xa5; 8];
                    bytes[1..4].copy_from_slice(&succ.to_le_bytes()[..3]);
                    bytes[4..7].copy_from_slice(&pred.to_le_bytes()[..3]);
                    h.bus.map(node - 1, &bytes, true);
                    if node == previous {
                        bytes[1..4].copy_from_slice(&following.to_le_bytes()[..3]);
                    } else if node == following {
                        bytes[4..7].copy_from_slice(&previous.to_le_bytes()[..3]);
                    }
                    expected.push((node, bytes));
                }
                h.run();
                h.guards(mask);
                for (node, bytes) in expected {
                    assert_eq!(&h.bus.ram[node as usize - 1..node as usize + 7], &bytes);
                }
                let writes: Vec<_> = h
                    .bus
                    .writes
                    .iter()
                    .copied()
                    .filter(|(a, _)| nodes.iter().any(|node| (*node - 1..*node + 7).contains(a)))
                    .collect();
                let expected_writes: Vec<_> = [(previous, following), (following + 3, previous)]
                    .into_iter()
                    .flat_map(|(at, value)| {
                        value.to_le_bytes()[..3]
                            .iter()
                            .copied()
                            .enumerate()
                            .map(move |(i, byte)| (at + i as u32, byte))
                            .collect::<Vec<_>>()
                    })
                    .collect();
                assert_eq!(writes, expected_writes);
            }
        }
    }
}

#[test]
fn field_offsets_near_and_beyond_y_range_preserve_accesses_and_addresses() {
    for offset in [3, 65532, 65533, 65535, 65536] {
        let field = 0x22ffffu32;
        // Individual embedded-array bounds are signed sixteen-bit constants.
        // Several small arrays exercise a large record displacement legally.
        let mut padding = String::new();
        let mut remaining = offset;
        let mut index = 0;
        while remaining != 0 {
            let size = remaining.min(16384);
            padding.push_str(&format!("BYTE ARRAY padding{index}({size}) "));
            remaining -= size;
            index += 1;
        }
        let source = format!(
            "TYPE Large=[{padding}BYTE POINTER link] \
             Large POINTER p BYTE POINTER result,replacement ADDRESS location \
             PROC Main() p=Large POINTER({}) replacement=BYTE POINTER($CD5678) \
             location=ADDRESS(@p.link) result=p.link p.link=replacement RETURN",
            field - offset,
        );
        for optimize in [false, true] {
            let image = compile(&source, optimize);
            let mut h = Harness::new(&image, &caller(image.entry), 0);
            // Only the three field bytes are mapped: widening must not access
            // padding or a fourth byte, even at a bank or Y-range boundary.
            h.bus.map(field, &[0x34, 0x12, 0xab], true);
            h.run();
            h.guards(0);
            assert_eq!(h.global(&image, "location", 3), field);
            assert_eq!(h.global(&image, "result", 3), 0xab1234);
            assert_eq!(h.bus.value(field, 3), 0xcd5678);
        }
    }
}

#[test]
fn volatile_three_byte_accesses_keep_exact_ascending_byte_traces() {
    use actionc_vm::native65816::Access;
    let source = "VOLATILE ADDRESS io=$D000 ADDRESS before,after,value \
        PROC Main() value=$CD5678 before=io io=value after=io RETURN";
    for optimize in [false, true] {
        let image = compile(source, optimize);
        let mut h = Harness::new(&image, &caller(image.entry), 0);
        h.bus.map(0xd000, &[0x34, 0x12, 0xab], true);
        h.bus.watched.extend(0xd000..0xd003);
        h.run();
        h.guards(0);
        assert_eq!(h.global(&image, "before", 3), 0xab1234);
        assert_eq!(h.global(&image, "after", 3), 0xcd5678);
        let trace: Vec<_> = h.bus.trace.iter().map(|(_, a, op)| (*a, *op)).collect();
        assert_eq!(
            trace,
            [
                (0xd000, Access::Read),
                (0xd001, Access::Read),
                (0xd002, Access::Read),
                (0xd000, Access::Write(0x78)),
                (0xd001, Access::Write(0x56)),
                (0xd002, Access::Write(0xcd)),
                (0xd000, Access::Read),
                (0xd001, Access::Read),
                (0xd002, Access::Read),
            ]
        );
    }
}

#[test]
fn named_record_pointer_casts_and_results_preserve_all_three_bytes() {
    let source = "TYPE Link=[Link POINTER next Link POINTER prev] \
        Link POINTER cursor,answer ADDRESS result \
        Link POINTER FUNC Identity(Link POINTER input) RETURN(Link POINTER(input)) \
        PROC Main() cursor=Link POINTER($12FFFE) \
        cursor.next=Link POINTER($AB1234) cursor.prev=Link POINTER(0) \
        answer=Identity(cursor.next) result=ADDRESS(answer) RETURN";
    for optimize in [false, true] {
        let image = compile(source, optimize);
        let mut h = Harness::new(&image, &caller(image.entry), 0);
        h.bus.map(0x12fff0, &[0xa5; 32], true);
        h.run();
        h.guards(0);
        assert_eq!(h.global(&image, "result", 3), 0xab1234);
        let mut expected = [0xa5; 32];
        expected[14..20].copy_from_slice(&[0x34, 0x12, 0xab, 0, 0, 0]);
        assert_eq!(&h.bus.ram[0x12fff0..0x130010], &expected);
    }
}

#[test]
fn absolute_array_indices_preserve_native_addresses_and_element_stride() {
    let source = "BYTE ARRAY bytes(1024)=$FF00 CARD ARRAY words(1024)=$A000 CARD index,result PROC Main() bytes(index)=37 words(index)=$BEEF result=CARD(bytes(index))+words(index) RETURN";
    for optimize in [false, true] {
        let image = compile(source, optimize);
        for index in [0u16, 255, 256, 1023] {
            let mut h = Harness::new(&image, &caller(image.entry), 0);
            h.bus.map(0xfef0, &[0xa5; 1056], true);
            h.bus.map(0x9ff0, &[0xa5; 2080], true);
            let at = symbol(&image, "index") as usize;
            h.bus.ram[at..at + 2].copy_from_slice(&index.to_le_bytes());
            h.run();
            h.guards(0);
            assert_eq!(h.global(&image, "result", 2), 0xbf14);
            let mut bytes = [0xa5; 1056];
            bytes[16 + usize::from(index)] = 37;
            assert_eq!(&h.bus.ram[0xfef0..0x10310], &bytes);
            let mut words = [0xa5; 2080];
            words[16 + usize::from(index) * 2..18 + usize::from(index) * 2]
                .copy_from_slice(&0xbeefu16.to_le_bytes());
            assert_eq!(&h.bus.ram[0x9ff0..0xa810], &words);
        }
    }
}

#[test]
fn logical_shifts_match_language_rules_at_every_scalar_width_and_count_boundary() {
    for (ty, bytes) in [
        ("BYTE", 1),
        ("CARD", 2),
        ("INT", 2),
        ("SIZE", 3),
        ("LONGCARD", 4),
        ("LONGINT", 4),
    ] {
        let bits = bytes * 8;
        let mask = ((1u64 << bits) - 1) as u32;
        for optimize in [false, true] {
            let image = compile(
                &format!(
                    "{ty} value,leftResult,rightResult {ty} count PROC Main() leftResult=value LSH count rightResult=value RSH count RETURN"
                ),
                optimize,
            );
            for count in [0, bits - 1, bits, bits + 1, 256, 0xffffff] {
                let mut h = Harness::new(&image, &caller(image.entry), 0);
                let value = 0x89abcdef & mask;
                let at = symbol(&image, "value") as usize;
                h.bus.ram[at..at + bytes].copy_from_slice(&value.to_le_bytes()[..bytes]);
                let count = count & mask as usize;
                let at = symbol(&image, "count") as usize;
                h.bus.ram[at..at + bytes].copy_from_slice(&(count as u32).to_le_bytes()[..bytes]);
                h.run();
                h.guards(0);
                assert_eq!(
                    h.global(&image, "leftResult", bytes),
                    if count >= bits {
                        0
                    } else {
                        value.wrapping_shl(count as u32) & mask
                    },
                    "{ty}/{count}"
                );
                assert_eq!(
                    h.global(&image, "rightResult", bytes),
                    if count >= bits { 0 } else { value >> count },
                    "{ty}/{count}"
                );
            }
        }
    }
}

#[test]
fn record_copy_local_initialization_and_overlapping_helpers_cross_banks() {
    let source = r#"
MODULE TEST
USE A816MEMORY
TYPE Packet=[BYTE first CARD word BYTE POINTER ptr]
Packet original,duplicate
CARD checksum
PROC Local()
 BYTE ARRAY values=[3 7 11]
 checksum=checksum+CARD(values(0))+CARD(values(1))+CARD(values(2))
 values(1)=99
RETURN
PROC Main()
 BYTE POINTER p,q
 original.first=$5A original.word=$BEEF original.ptr=BYTE POINTER($AB1234)
 duplicate=original
 Local() Local()
 p=BYTE POINTER($12FFF0) q=BYTE POINTER($12FFF5)
 A816MEMORY.Move(q,p,24)
 A816MEMORY.Move(p,q,24)
 A816MEMORY.Clear(q,17)
RETURN
ENDMODULE
"#;
    for optimize in [false, true] {
        let image = compile(source, optimize);
        let mut h = Harness::new(&image, &caller(image.entry), 0);
        let initial: Vec<u8> = (0..64).map(|n| n + 10).collect();
        h.bus.map(0x12fff0, &initial, true);
        h.run();
        h.guards(0);
        assert_eq!(h.global(&image, "checksum", 2), 42);
        let a = symbol(&image, "original") as usize;
        let b = symbol(&image, "duplicate") as usize;
        assert_eq!(&h.bus.ram[a..a + 8], &h.bus.ram[b..b + 8]);
        assert_eq!(
            &h.bus.ram[b..b + 8],
            &[0x5a, 0, 0xef, 0xbe, 0x34, 0x12, 0xab, 0]
        );
        let mut expected = initial;
        expected.copy_within(0..24, 5);
        expected.copy_within(5..29, 0);
        expected[5..22].fill(0);
        assert_eq!(&h.bus.ram[0x12fff0..0x130030], expected);
    }
}

#[test]
fn pointer_offsets_keep_wide_magnitudes_and_signed_narrow_displacements() {
    let source = "BYTE POINTER base,far,back,subtract INT negative SIZE large PROC Main() base=BYTE POINTER($12FFFE) negative=-2 large=$10004 far=base+large back=base+negative subtract=base-negative RETURN";
    for optimize in [false, true] {
        let image = compile(source, optimize);
        let mut h = Harness::new(&image, &caller(image.entry), 0);
        h.run();
        h.guards(0);
        assert_eq!(h.global(&image, "far", 3), 0x140002);
        assert_eq!(h.global(&image, "back", 3), 0x12fffc);
        assert_eq!(h.global(&image, "subtract", 3), 0x130000);
    }
}
