//! CPU measurements of real TN routines; disk/screen services are substituted.
//! No cycle counts are correctness assertions. Run with --nocapture for CSV.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, VmRunHooks};

#[path = "support/tn_symbols.rs"]
mod symbols;
use symbols::{global_address, routine_address};
#[allow(dead_code)]
#[path = "support/tn_directory.rs"]
mod directory;
#[allow(dead_code)]
#[path = "support/tn_layout.rs"]
mod layout;
#[path = "support/tn_machine.rs"]
mod machine;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn timed<H: VmRunHooks<Error = String>>(
    vm: CompilerVm,
    hooks: &mut H,
    address: u16,
    args: &[u8],
    mode: CompileMode,
    workload: &str,
    count: usize,
    private: &BTreeSet<u16>,
) -> CompilerVm {
    let start = vm.cpu().cycles();
    let mut tally = SpillTally {
        inner: hooks,
        private,
        pending: None,
        accesses: 0,
    };
    let vm = machine::call(vm, &mut tally, address, args);
    println!(
        "{mode:?},{workload},{count},{},{}",
        vm.cpu().cycles() - start,
        tally.accesses
    );
    vm
}

// Count data operand accesses to emitted compiler spill cells. This excludes
// canonical params/locals, fixed runtime scratch, stack and pointer-fetch bytes.
// It is a deliberately bounded census, not a count of all private bus traffic.
fn emitted_spill_addresses(listing: &str) -> BTreeSet<u16> {
    let mut result = BTreeSet::new();
    let mut spill = false;
    for line in listing.lines() {
        if line.starts_with("; =====") {
            spill = false;
        }
        if let Some(label) = line.strip_suffix(':') {
            spill = label.starts_with("spill_");
        }
        if let Some((label, address)) = line.split_once(" = $") {
            if label.starts_with("spill_") {
                if let Ok(address) = u16::from_str_radix(address, 16) {
                    result.insert(address);
                }
            }
        }
        if spill {
            if let Some((address, bytes)) = line
                .split_once("; $")
                .and_then(|(_, rest)| rest.split_once(':'))
            {
                if let Ok(address) = u16::from_str_radix(address, 16) {
                    for lane in 0..bytes.split_whitespace().take_while(|byte| byte.len() == 2 && byte.bytes().all(|c| c.is_ascii_hexdigit())).count() {
                        result.insert(address.wrapping_add(lane as u16));
                    }
                }
            }
        }
    }
    result
}

struct SpillTally<'a, H> {
    inner: &'a mut H,
    private: &'a BTreeSet<u16>,
    pending: Option<(u16, usize)>,
    accesses: usize,
}
impl<H: VmRunHooks<Error = String>> VmRunHooks for SpillTally<'_, H> {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        self.inner.before_step(vm)?;
        self.pending = spill_operand(vm);
        Ok(())
    }
    fn after_step(&mut self, vm: &CompilerVm, step: &actionc_vm::CpuStep) -> Result<(), String> {
        if let Some((address, accesses)) = self.pending.take() {
            if self.private.contains(&address) {
                self.accesses += accesses;
            }
        }
        self.inner.after_step(vm, step)
    }
    fn stop_reason(
        &self,
        vm: &CompilerVm,
        step: &actionc_vm::CpuStep,
    ) -> Option<actionc_vm::StopReason> {
        self.inner.stop_reason(vm, step)
    }
}
fn spill_operand(vm: &CompilerVm) -> Option<(u16, usize)> {
    let r = vm.cpu().registers();
    // Generated TN code lives in RAM; resident services are outside the census.
    if !(0x2C00..0xA000).contains(&r.pc) {
        return None;
    }
    let ram = vm.bus().ram();
    let op = ram.read(r.pc);
    let arg = ram.read(r.pc.wrapping_add(1));
    let absolute = ram.read_word(r.pc.wrapping_add(1));
    if op & 3 == 1 {
        let address = match op & 0x1C {
            0x00 => {
                let p = arg.wrapping_add(r.x);
                u16::from_le_bytes([
                    ram.read(u16::from(p)),
                    ram.read(u16::from(p.wrapping_add(1))),
                ])
            }
            0x04 => u16::from(arg),
            0x0C => absolute,
            0x10 => u16::from_le_bytes([
                ram.read(u16::from(arg)),
                ram.read(u16::from(arg.wrapping_add(1))),
            ])
            .wrapping_add(u16::from(r.y)),
            0x14 => u16::from(arg.wrapping_add(r.x)),
            0x18 => absolute.wrapping_add(u16::from(r.y)),
            0x1C => absolute.wrapping_add(u16::from(r.x)),
            _ => return None,
        };
        return Some((address, 1));
    }
    let (address, accesses) = match op {
        0xA4 | 0xA6 | 0x84 | 0x86 | 0xC4 | 0xE4 | 0x24 => (u16::from(arg), 1),
        0xB4 | 0x94 => (u16::from(arg.wrapping_add(r.x)), 1),
        0xB6 | 0x96 => (u16::from(arg.wrapping_add(r.y)), 1),
        0xAC | 0xAE | 0x8C | 0x8E | 0xCC | 0xEC | 0x2C => (absolute, 1),
        0xBC => (absolute.wrapping_add(u16::from(r.x)), 1),
        0xBE => (absolute.wrapping_add(u16::from(r.y)), 1),
        0x06 | 0x26 | 0x46 | 0x66 | 0xC6 | 0xE6 => (u16::from(arg), 2),
        0x16 | 0x36 | 0x56 | 0x76 | 0xD6 | 0xF6 => (u16::from(arg.wrapping_add(r.x)), 2),
        0x0E | 0x2E | 0x4E | 0x6E | 0xCE | 0xEE => (absolute, 2),
        0x1E | 0x3E | 0x5E | 0x7E | 0xDE | 0xFE => (absolute.wrapping_add(u16::from(r.x)), 2),
        _ => return None,
    };
    Some((address, accesses))
}

#[test]
fn spill_counter_measures_executed_operand_reads_and_writes() {
    struct Hooks;
    impl VmRunHooks for Hooks {
        type Error = String;
    }
    let mut vm = CompilerVm::default();
    vm.bus_mut()
        .ram_mut()
        .map(
            0x3000,
            &[
                0xA9, 1, 0x8D, 0, 0x54, 0xA2, 1, 0xBD, 0xFF, 0x53, 0xFE, 0xFF, 0x53, 0x2C, 0, 0x54,
                0x60,
            ],
        )
        .unwrap();
    let private = BTreeSet::from([0x5400]);
    let mut tally = SpillTally {
        inner: &mut Hooks,
        private: &private,
        pending: None,
        accesses: 0,
    };
    let vm = machine::call(vm, &mut tally, 0x3000, &[]);
    assert_eq!(tally.accesses, 5);
    assert_eq!(vm.bus().ram().read(0x5400), 2);
    assert_eq!(
        emitted_spill_addresses(
            "; ===== DATA spill0 $5400 =====\nspill_demo_0:\n .BYTE $00 ; $5400: 00\n"
        ),
        private
    );
    assert_eq!(
        emitted_spill_addresses(
            "; ===== DATA spill0 $5400 =====\r\nspill_demo_0:\r\n .BYTE $00 ; $5400: 00\r\n"
        ),
        private
    );
}

fn words(values: &[u16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

#[test]
fn tn_cpu_workloads_preserve_rows_order_and_selection() {
    println!(
        "backend,workload,count,cpu_cycles_including_call_wrapper,emitted_spill_operand_accesses"
    );
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        let compiled = compile_file(
            &root().join("samples/tn/modern/TN.ACT"),
            &CompileOptions::for_mode(mode).with_runtime(Runtime::ActionCart),
        )
        .unwrap();
        let listing = compiled.source_listing();
        let private = emitted_spill_addresses(&listing);
        let routine = |name| routine_address(&listing, name);
        let global = |name| global_address(&listing, name);
        for count in [0usize, 1, 64] {
            let files = directory::files(count);
            let mut expected = files.clone();
            expected.sort_by_key(|f| (!f.directory, f.name.clone(), f.extension.clone()));
            let mut input: Vec<_> = files.iter().map(directory::File::input).collect();
            let mut summary = b"999 FREE SECTORS\x9B".to_vec();
            summary.resize(19, 155);
            input.push(summary);
            let mut hooks = directory::Directory {
                entries: [
                    "Path",
                    "DrawWinFrame",
                    "Inv",
                    "Close",
                    "Open",
                    "Input",
                    "PutImage",
                ]
                .into_iter()
                .map(|name| (routine(name), name))
                .collect(),
                ioerr: global("ioerr"),
                input,
                row: 0,
                images: vec![],
                names: vec![],
            };
            let mut vm = machine::load(&compiled);
            for (address, value) in [(0x700, b'M'), (0x76F, 0xA9), (0x70A, 0xFF), (0x70B, 2)] {
                vm.bus_mut().ram_mut().write(address, value);
            }
            vm = machine::call(vm, &mut hooks, routine("InitPanels"), &[]);
            let batch = vm.bus().ram().read_word(global("currentbatch"));
            let tags = vm.bus().ram().read_word(global("currenttags"));
            // Reverse the already sorted logical order, independently of slots.
            let order = machine::bytes(&vm, batch + layout::ORDER, count);
            if count != 0 {
                vm.bus_mut()
                    .ram_mut()
                    .map(
                        batch + layout::ORDER,
                        &order.iter().copied().rev().collect::<Vec<_>>(),
                    )
                    .unwrap();
            }
            vm = timed(
                vm,
                &mut hooks,
                routine("SortBatch"),
                &words(&[batch]),
                mode,
                "sort_reversed",
                count,
                &private,
            );
            assert_eq!(machine::bytes(&vm, batch + layout::ORDER, count), order);
            hooks.images.clear();
            vm = timed(
                vm,
                &mut hooks,
                routine("Draw"),
                &words(&[0]),
                mode,
                "draw",
                count,
                &private,
            );
            assert_eq!(
                &hooks.images[1..],
                expected
                    .iter()
                    .take(16)
                    .map(|f| f.row(false))
                    .collect::<Vec<_>>()
            );
            vm = timed(
                vm,
                &mut hooks,
                routine("ToggleAllTags"),
                &words(&[tags]),
                mode,
                "tag_all",
                count,
                &private,
            );
            assert_eq!(vm.bus().ram().read_word(tags + 6), count as u16);
            vm = timed(
                vm,
                &mut hooks,
                routine("ToggleTag"),
                &words(&[tags, 0]),
                mode,
                "toggle_tag",
                count,
                &private,
            );
            assert_eq!(vm.bus().ram().read(0xA0), if count == 0 { 2 } else { 1 });
            assert_eq!(
                vm.bus().ram().read_word(tags + 6),
                count.saturating_sub(1) as u16
            );
            if count != 0 {
                vm = machine::call(vm, &mut hooks, routine("ToggleTag"), &words(&[tags, 0]));
            }
            // Use guarded fixture-owned bitmap RAM for the paged workload.
            vm.bus_mut().ram_mut().map(0x800, &[0xA5; 514]).unwrap();
            vm = machine::call(
                vm,
                &mut hooks,
                routine("TagsInit"),
                &words(&[tags, 0x801, 4096]),
            );
            vm = machine::call(
                vm,
                &mut hooks,
                routine("BatchBegin"),
                &words(&[batch, 1024]),
            );
            let mut args = words(&[batch, tags]);
            args.extend([64, 0]);
            vm = timed(
                vm,
                &mut hooks,
                routine("BatchPublish"),
                &args,
                mode,
                "publish_prefix",
                1088,
                &private,
            );
            assert_eq!(vm.bus().ram().read(batch + layout::VALID), 1);
            assert_eq!(vm.bus().ram().read_word(tags + 4), 1088);
            assert_eq!(vm.bus().ram().read_word(tags + 6), 0);
            assert_eq!(vm.bus().ram().read(0x800), 0xA5);
            assert_eq!(vm.bus().ram().read(0xA01), 0xA5);
        }
    }
}
