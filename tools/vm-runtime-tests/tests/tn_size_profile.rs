//! CPU measurements of real TN routines; disk/screen services are substituted.
//! No cycle counts are correctness assertions. Run with --nocapture for CSV.
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
) -> CompilerVm {
    let start = vm.cpu().cycles();
    let vm = machine::call(vm, hooks, address, args);
    println!("{mode:?},{workload},{count},{}", vm.cpu().cycles() - start);
    vm
}

fn words(values: &[u16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

#[test]
fn tn_cpu_workloads_preserve_rows_order_and_selection() {
    println!("backend,workload,count,cpu_cycles_including_call_wrapper");
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        let compiled = compile_file(
            &root().join("samples/tn/modern/TN.ACT"),
            &CompileOptions::for_mode(mode).with_runtime(Runtime::ActionCart),
        )
        .unwrap();
        let listing = compiled.source_listing();
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
            );
            assert_eq!(vm.bus().ram().read_word(tags + 6), count as u16);
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
            );
            assert_eq!(vm.bus().ram().read(batch + layout::VALID), 1);
            assert_eq!(vm.bus().ram().read_word(tags + 4), 1088);
            assert_eq!(vm.bus().ram().read_word(tags + 6), 0);
            assert_eq!(vm.bus().ram().read(0x800), 0xA5);
            assert_eq!(vm.bus().ram().read(0xA01), 0xA5);
        }
    }
}
