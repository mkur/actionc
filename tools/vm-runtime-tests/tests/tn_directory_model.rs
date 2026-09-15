use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, VmRunHooks};
use std::path::{Path, PathBuf};

#[path = "support/tn_machine.rs"]
mod machine;
#[path = "support/tn_symbols.rs"]
mod symbols;
use symbols::{global_address, routine_address};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
struct Source(PathBuf);
impl Drop for Source {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
struct Hooks;
impl VmRunHooks for Hooks {
    type Error = String;
    fn before_step(&mut self, _vm: &mut CompilerVm) -> Result<(), String> {
        Ok(())
    }
}
fn args(words: &[u16], tail: &[u8]) -> Vec<u8> {
    words
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .chain(tail.iter().copied())
        .collect()
}

#[test]
fn directory_cache_and_selection_are_independent_and_bounded() {
    let prefix = "ORG $2C00\nSET $E=$E6\nSET $F=0\nBYTE POINTER screen\nCARD POINTER allocp\nSET $E=$2C00\nSET $491=$2C00\n";
    let production = std::fs::read_to_string(root().join("samples/tn/modern/DIR.ACT"))
        .unwrap()
        .replace("\r\n", "\n");
    let fixture = include_str!("../fixtures/tn/directory-model.act").replace("\r\n", "\n");
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        for crlf in [false, true] {
            let source = Source(std::env::temp_dir().join(format!(
                "actionc-tn-directory-{}-{mode:?}-{crlf}.act",
                std::process::id()
            )));
            let mut text = format!("{prefix}{production}\n{fixture}");
            if crlf {
                text = text.replace('\n', "\r\n");
            }
            std::fs::write(&source.0, text).unwrap();
            let compiled = compile_file(
                &source.0,
                &CompileOptions::for_mode(mode).with_runtime(Runtime::ActionCart),
            )
            .unwrap();
            let listing = compiled.source_listing();
            let global = |s| global_address(&listing, s);
            let routine = |s| routine_address(&listing, s);
            let mut vm = machine::load(&compiled);
            let mut hooks = Hooks;
            // Obtain backing addresses through the real binding routine:
            // small inline arrays and large descriptors have different homes.
            vm = machine::call(vm, &mut hooks, routine("Start"), &[]);
            let left_bits = vm.bus().ram().read_word(global("left")) - 1;
            let right_bits = vm.bus().ram().read_word(global("right")) - 1;
            vm.bus_mut().ram_mut().map(left_bits, &[0xA5; 514]).unwrap();
            vm.bus_mut().ram_mut().map(right_bits, &[0xA5; 10]).unwrap();
            vm = machine::call(vm, &mut hooks, routine("Start"), &[]);
            let left = global("left");
            let right = global("right");
            assert_eq!(vm.bus().ram().read_word(left), left_bits + 1);
            assert_eq!(vm.bus().ram().read_word(right), right_bits + 1);
            assert_eq!(vm.bus().ram().read_word(left + 2), 4096, "initial capacity");
            vm = machine::call(
                vm,
                &mut hooks,
                routine("TagsExtent"),
                &args(&[left, 4096], &[1]),
            );
            assert_eq!(
                vm.bus().ram().read_word(left + 4),
                4096,
                "published extent: {:?}",
                machine::bytes(&vm, left, 14)
            );
            let selected = [0, 7, 8, 63, 64, 255, 256, 1023, 1024, 4095];
            let mut expected = vec![0; 512];
            for ordinal in selected {
                vm = machine::call(
                    vm,
                    &mut hooks,
                    routine("SetTagged"),
                    &args(&[left, ordinal], &[1]),
                );
                assert_eq!(vm.cpu().registers().a, 1, "{mode:?}/{crlf}/{ordinal}");
                expected[ordinal as usize / 8] |= 1 << (ordinal % 8);
            }
            assert_eq!(machine::bytes(&vm, left_bits + 1, 512), expected);
            assert_eq!(vm.bus().ram().read_word(left + 6), selected.len() as u16);
            assert_eq!(machine::bytes(&vm, right_bits + 1, 8), [0; 8]);
            for ordinal in selected {
                vm = machine::call(
                    vm,
                    &mut hooks,
                    routine("NextTagged"),
                    &args(&[left, ordinal, global("found")], &[]),
                );
                assert_eq!(
                    vm.cpu().registers().a,
                    1,
                    "next from {ordinal}, found {}, state {:?}",
                    vm.bus().ram().read_word(global("found")),
                    machine::bytes(&vm, left, 14)
                );
                assert_eq!(vm.bus().ram().read_word(global("found")), ordinal);
            }
            for ordinal in [4096, 65535] {
                vm = machine::call(
                    vm,
                    &mut hooks,
                    routine("NextTagged"),
                    &args(&[left, ordinal, global("found")], &[]),
                );
                assert_eq!(vm.cpu().registers().a, 0);
                vm = machine::call(
                    vm,
                    &mut hooks,
                    routine("SetTagged"),
                    &args(&[left, ordinal], &[1]),
                );
                assert_eq!(vm.cpu().registers().a, 2);
            }
            let cache = vm.bus().ram().read_word(global("cache"));
            for first in [0, 64, 256, 1024, 4032, 0] {
                vm = machine::call(
                    vm,
                    &mut hooks,
                    routine("FakeRead"),
                    &args(&[first], &[64, 0]),
                );
                assert_eq!(vm.bus().ram().read(global("result")), 1);
                assert_eq!(
                    machine::bytes(&vm, cache + 1280, 5),
                    [first as u8, (first >> 8) as u8, 64, 1, 0]
                );
                vm = machine::call(
                    vm,
                    &mut hooks,
                    routine("CachedEntry"),
                    &args(&[cache, first + 63], &[]),
                );
                let p = vm.bus().ram().read_word(0xA0);
                assert_eq!(p, cache + 63 * 20);
                assert_eq!(vm.bus().ram().read_word(p), first + 63);
                assert_eq!(
                    machine::bytes(&vm, left_bits + 1, 512),
                    expected,
                    "eviction lost tags"
                );
            }
            for (top, start) in [
                (48u16, 0),
                (49, 49),
                (63, 63),
                (64, 64),
                (255, 255),
                (256, 256),
            ] {
                vm = machine::call(vm, &mut hooks, routine("BatchStart"), &top.to_le_bytes());
                assert_eq!(vm.bus().ram().read_word(0xA0), start);
            }
            vm = machine::call(vm, &mut hooks, routine("TagsReset"), &left.to_le_bytes());
            vm = machine::call(
                vm,
                &mut hooks,
                routine("ToggleAllTags"),
                &left.to_le_bytes(),
            );
            vm = machine::call(vm, &mut hooks, routine("TagsReady"), &left.to_le_bytes());
            assert_eq!(
                vm.cpu().registers().a,
                5,
                "unknown tag-all cannot operate yet"
            );
            vm = machine::call(
                vm,
                &mut hooks,
                routine("FakeRead"),
                &args(&[1024], &[64, 0]),
            );
            vm = machine::call(
                vm,
                &mut hooks,
                routine("SetTagged"),
                &args(&[left, 1024], &[0]),
            );
            vm = machine::call(vm, &mut hooks, routine("FakeRead"), &args(&[0], &[64, 0]));
            vm = machine::call(
                vm,
                &mut hooks,
                routine("TagsExtent"),
                &args(&[left, 1088], &[1]),
            );
            assert_eq!(vm.bus().ram().read_word(left + 6), 1087);
            assert_eq!(machine::bytes(&vm, left_bits + 137, 376), vec![0; 376]);
            vm = machine::call(vm, &mut hooks, routine("TagsReady"), &left.to_le_bytes());
            assert_eq!(vm.cpu().registers().a, 1);
            vm = machine::call(
                vm,
                &mut hooks,
                routine("TagsExtent"),
                &args(&[left, 4097], &[1]),
            );
            vm = machine::call(vm, &mut hooks, routine("TagsReady"), &left.to_le_bytes());
            assert_eq!(
                vm.cpu().registers().a,
                2,
                "tag-all must report its capacity limit"
            );
            vm = machine::call(vm, &mut hooks, routine("TagsReset"), &left.to_le_bytes());
            assert_eq!(machine::bytes(&vm, left_bits + 1, 512), vec![0; 512]);
            vm = machine::call(vm, &mut hooks, routine("FakeRead"), &args(&[0], &[65, 0]));
            assert_eq!(vm.bus().ram().read(global("result")), 2);
            // Cache's entries occupy 1280 bytes; its validity byte follows
            // first (CARD) and count (BYTE). This checks the public data layout.
            assert_eq!(vm.bus().ram().read(cache + 1283), 0);
            for (address, count) in [(left_bits, 514), (right_bits, 10)] {
                assert_eq!(vm.bus().ram().read(address), 0xA5);
                assert_eq!(vm.bus().ram().read(address + count - 1), 0xA5);
            }
        }
    }
}
