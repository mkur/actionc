//! Ordinary-RAM traversal experiments, not authorization for alias-sensitive rewrites.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, VmRunHooks};
use std::path::{Path, PathBuf};

#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;
#[path = "support/tn_symbols.rs"]
mod symbols;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[derive(Default)]
struct Reads {
    base: u16,
    len: u16,
    addresses: Vec<u16>,
    calls: usize,
    pointer_writes: usize,
}
impl VmRunHooks for Reads {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let r = vm.cpu().registers();
        let ram = vm.bus().ram();
        let op = ram.read(r.pc);
        let arg = ram.read(r.pc.wrapping_add(1));
        if op == 0x20 {
            self.calls += 1;
        }
        if matches!(op, 0x85 | 0x84 | 0x86) && (0xAA..=0xAF).contains(&arg) {
            self.pointer_writes += 1;
        }
        if matches!(op, 0xB1 | 0xD1) {
            let address = ram.read_word(u16::from(arg)).wrapping_add(u16::from(r.y));
            if address.wrapping_sub(self.base) < self.len {
                self.addresses.push(address);
            }
        }
        Ok(())
    }
}

fn map_word(vm: &mut CompilerVm, address: u16, value: u16) {
    vm.bus_mut()
        .ram_mut()
        .map(address, &value.to_le_bytes())
        .unwrap();
}

#[test]
fn bitmap_traversal_variants_match_host_counts_and_preserve_guards() {
    let source = std::fs::read_to_string(root().join("surveys/tn/reproducers/bitmap-scans.act"))
        .unwrap()
        .replace("\r\n", "\n");
    println!(
        "bitmap,backend,runtime,routine,capacity,limit,pattern,cycles,bitmap_reads,jsr,pointer_writes"
    );
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let mut lf_image = None;
            for crlf in [false, true] {
                let text = if crlf {
                    source.replace('\n', "\r\n")
                } else {
                    source.clone()
                };
                let file = support::Source::new(&text);
                let compiled = compile_file(
                    &file.0,
                    &CompileOptions::for_mode(mode)
                        .with_runtime(runtime)
                        .with_origin(0x3000),
                )
                .unwrap();
                if let Some(lf) = &lf_image {
                    assert_eq!(compiled.object_bytes(), lf);
                } else {
                    lf_image = Some(compiled.object_bytes().to_vec());
                }
                let listing = compiled.source_listing();
                let (mut vm, profile) = support::vm_for(runtime);
                vm.load_atari_object_for_execution(profile, compiled.object_bytes())
                    .unwrap();
                for (capacity, limits) in [
                    (0u16, vec![0, 1]),
                    (8, vec![0, 1, 7, 8, 9]),
                    (64, vec![0, 1, 7, 8, 9, 63, 64, 65]),
                    (4096, vec![255, 256, 257, 1088, 1089, 4095, 4096, 4097]),
                    // The largest valid multiple-of-eight CARD capacity.
                    (65528, vec![65527, 65528, 65535]),
                ] {
                    for limit in limits {
                        for pattern in [0u8, 1, 2] {
                            // Focus the expensive wide vectors on distinct byte/count boundaries.
                            if capacity == 65528 && (crlf || pattern == 2) {
                                continue;
                            }
                            let len = usize::from(capacity / 8);
                            let bits = (0..len)
                                .map(|n| match pattern {
                                    0 => 0,
                                    1 => 255,
                                    _ => (n as u8).wrapping_mul(37).wrapping_add(129),
                                })
                                .collect::<Vec<_>>();
                            let visited = usize::from(limit.min(capacity));
                            let expected = (0..visited)
                                .filter(|n| bits[n / 8] & (1 << (n % 8)) != 0)
                                .count() as u16;
                            let base = if pattern == 2 { 0x50F9 } else { 0x5001 };
                            for name in ["BitByBit", "CachedBits", "ByteShift", "ByteNibbles"] {
                                let record = 0x4801u16;
                                vm.bus_mut().ram_mut().map(0x4800, &[0xA5; 10]).unwrap();
                                map_word(&mut vm, record, base);
                                map_word(&mut vm, record + 2, capacity);
                                map_word(&mut vm, record + 4, limit);
                                map_word(&mut vm, record + 6, 0xFFFF);
                                let mut guarded = vec![0xA5];
                                guarded.extend(&bits);
                                guarded.push(0x5A);
                                vm.bus_mut().ram_mut().map(base - 1, &guarded).unwrap();
                                let before = machine::bytes(&vm, 0x4800, 10);
                                let cycles = vm.cpu().cycles();
                                let mut reads = Reads {
                                    base: base - 1,
                                    len: (len + 2) as u16,
                                    ..Reads::default()
                                };
                                vm = machine::call(
                                    vm,
                                    &mut reads,
                                    symbols::routine_address(&listing, name),
                                    &record.to_le_bytes(),
                                );
                                assert_eq!(
                                    vm.bus().ram().read_word(record + 6),
                                    expected,
                                    "{mode:?}/{runtime:?}/{crlf}/{name}/{capacity}/{limit}/{pattern}"
                                );
                                assert_eq!(machine::bytes(&vm, base - 1, len + 2), guarded);
                                let mut after = before;
                                after[7..9].copy_from_slice(&expected.to_le_bytes());
                                assert_eq!(machine::bytes(&vm, 0x4800, 10), after);
                                let expected_reads = if name.starts_with("Byte") {
                                    (0..visited.div_ceil(8))
                                        .map(|n| base + n as u16)
                                        .collect::<Vec<_>>()
                                } else {
                                    (0..visited)
                                        .map(|n| base + (n / 8) as u16)
                                        .collect::<Vec<_>>()
                                };
                                assert_eq!(reads.addresses, expected_reads);
                                if !crlf && [0, 1, 64, 1088, 1089, 65528].contains(&limit) {
                                    println!(
                                        "bitmap,{mode:?},{runtime:?},{name},{capacity},{limit},{pattern},{},{},{},{}",
                                        vm.cpu().cycles() - cycles,
                                        reads.addresses.len(),
                                        reads.calls,
                                        reads.pointer_writes
                                    );
                                }
                            }
                        }
                    }
                }
                // Aliasing the backing with the result distinguishes the baseline
                // from a captured-byte scan. Such source experiments do not prove
                // a general compiler rewrite safe for arbitrary pointers.
                for (name, expected) in [("BitByBit", 0), ("ByteNibbles", 8)] {
                    let record = 0x4801;
                    map_word(&mut vm, record, record + 6);
                    map_word(&mut vm, record + 2, 8);
                    map_word(&mut vm, record + 4, 8);
                    map_word(&mut vm, record + 6, 0xFFFF);
                    vm = machine::call(
                        vm,
                        &mut Reads::default(),
                        symbols::routine_address(&listing, name),
                        &record.to_le_bytes(),
                    );
                    assert_eq!(vm.bus().ram().read_word(record + 6), expected);
                }
            }
        }
    }
}

#[test]
fn production_tail_bound_matches_the_uncached_loop_in_both_runtimes() {
    let current = std::fs::read_to_string(root().join("samples/tn/modern/DIR.ACT"))
        .unwrap()
        .replace("\r\n", "\n");
    assert_eq!(current.matches("byteLimit=capacity RSH 3\n").count(), 1);
    let previous = current
        .replace("CARD ordinal,byteLimit\n", "CARD ordinal\n")
        .replace("    byteLimit=capacity RSH 3\n", "")
        .replace(
            "WHILE ordinal<byteLimit DO",
            "WHILE ordinal<(capacity RSH 3) DO",
        );
    let driver = "Selection POINTER descriptor\nCARD extent\nBYTE ended\nPROC Main()\n  TagsExtent(descriptor,extent,ended)\nRETURN\n";
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for crlf in [false, true] {
                for source in [&previous, &current] {
                    let text = format!("{source}\n{driver}");
                    let text = if crlf {
                        text.replace('\n', "\r\n")
                    } else {
                        text
                    };
                    let file = support::Source::new(&text);
                    let compiled = compile_file(
                        &file.0,
                        &CompileOptions::for_mode(mode)
                            .with_runtime(runtime)
                            .with_origin(0x3000),
                    )
                    .unwrap();
                    let listing = compiled.source_listing();
                    let (mut vm, profile) = support::vm_for(runtime);
                    vm.load_atari_object_for_execution(profile, compiled.object_bytes())
                        .unwrap();
                    for capacity in [8u16, 64, 4096, 65528] {
                        let limits = if capacity == 65528 {
                            if !crlf && mode == CompileMode::Mir6502 {
                                vec![0, 65527, 65535]
                            } else {
                                vec![0]
                            }
                        } else {
                            vec![0, 1, 7, 8, 9, capacity - 1, capacity, capacity + 1]
                        };
                        for extent in limits {
                            for complete in [0u8, 1] {
                                if capacity == 65528 && extent == 0 && complete == 0 {
                                    continue;
                                }
                                let record = 0x4801u16;
                                let base = 0x5001u16;
                                let len = usize::from(capacity / 8);
                                let mut expected = vec![0xFF; len];
                                if complete != 0 && extent < capacity {
                                    for n in usize::from(extent)..usize::from(capacity) {
                                        expected[n / 8] &= !(1 << (n % 8));
                                    }
                                }
                                let mut metadata = vec![0xA5; 16];
                                for (offset, value) in
                                    [(1, base), (3, capacity), (5, 0), (7, 0xFFFF), (9, 0x1234)]
                                {
                                    metadata[offset..offset + 2]
                                        .copy_from_slice(&value.to_le_bytes());
                                }
                                metadata[11..15].copy_from_slice(&[0, 1, 1, 0]);
                                vm.bus_mut().ram_mut().map(record - 1, &metadata).unwrap();
                                let mut bitmap = vec![0xA5];
                                bitmap.extend(vec![0xFF; len]);
                                bitmap.push(0x5A);
                                vm.bus_mut().ram_mut().map(base - 1, &bitmap).unwrap();
                                let args = [record, extent]
                                    .into_iter()
                                    .flat_map(u16::to_le_bytes)
                                    .chain([complete])
                                    .collect::<Vec<_>>();
                                vm = machine::call(
                                    vm,
                                    &mut Reads::default(),
                                    symbols::routine_address(&listing, "TagsExtent"),
                                    &args,
                                );
                                metadata[5..7].copy_from_slice(&extent.to_le_bytes());
                                metadata[7..9].copy_from_slice(&extent.min(capacity).to_le_bytes());
                                metadata[11] = complete;
                                metadata[14] = u8::from(extent > capacity);
                                assert_eq!(
                                    machine::bytes(&vm, record - 1, 16),
                                    metadata,
                                    "{mode:?}/{runtime:?}/{crlf}/{capacity}/{extent}/{complete}"
                                );
                                bitmap[1..=len].copy_from_slice(&expected);
                                assert_eq!(machine::bytes(&vm, base - 1, len + 2), bitmap);
                            }
                        }
                    }
                }
            }
        }
    }
}
