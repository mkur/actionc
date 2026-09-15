use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use std::path::{Path, PathBuf};

#[path = "support/tn_machine.rs"]
mod machine;
#[path = "support/tn_symbols.rs"]
mod symbols;
use symbols::{global_address, routine_address};
#[allow(dead_code)]
#[path = "support/tn_directory.rs"]
mod directory;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
struct Source(PathBuf);
impl Drop for Source {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
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
fn mydos_reader_renders_and_resolves_a_complete_bounded_batch() {
    let mut source_text = "ORG $2C00\n".to_owned();
    for name in ["LIB.ACT", "DIR.ACT", "MYDOS.ACT"] {
        source_text.push_str(
            &std::fs::read_to_string(root().join("samples/tn/modern").join(name))
                .unwrap()
                .replace("\r\n", "\n"),
        );
        source_text.push('\n');
    }
    source_text.push_str(&include_str!("../fixtures/tn/mydos-reader.act").replace("\r\n", "\n"));
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        for crlf in [false, true] {
            let source = Source(std::env::temp_dir().join(format!(
                "actionc-tn-mydos-{}-{mode:?}-{crlf}.act",
                std::process::id()
            )));
            std::fs::write(
                &source.0,
                if crlf {
                    source_text.replace('\n', "\r\n")
                } else {
                    source_text.clone()
                },
            )
            .unwrap();
            let compiled = compile_file(
                &source.0,
                &CompileOptions::for_mode(mode).with_runtime(Runtime::ActionCart),
            )
            .unwrap();
            let listing = compiled.source_listing();
            let global = |s| global_address(&listing, s);
            let routine = |s| routine_address(&listing, s);
            for count in [0, 1, 63, 64, 65] {
                let files = directory::files(count);
                let mut expected = files.clone();
                expected.sort_by_key(|f| (!f.directory, f.name.clone(), f.extension.clone()));
                let mut input = files.iter().map(directory::File::input).collect::<Vec<_>>();
                let mut summary = b"999 FREE SECTORS\x9B".to_vec();
                summary.resize(19, 155);
                input.push(summary);
                let mut hooks = directory::Directory {
                    entries: ["Close", "Open", "Input"]
                        .into_iter()
                        .map(|n| (routine(n), n))
                        .collect(),
                    ioerr: global("ioerr"),
                    input,
                    row: 0,
                    images: vec![],
                    names: vec![],
                };
                let mut vm = machine::load(&compiled);
                vm = machine::call(vm, &mut hooks, routine("ReaderInit"), &[]);
                let cache = vm.bus().ram().read_word(global("cache"));
                let name = vm.bus().ram().read_word(global("nameaddress"));
                let row = vm.bus().ram().read_word(global("rowaddress"));
                for (start, length) in [(cache - 1, 1369), (name - 1, 17), (row - 1, 20)] {
                    vm.bus_mut()
                        .ram_mut()
                        .map(start, &vec![0xA5; length])
                        .unwrap();
                }
                vm = machine::call(
                    vm,
                    &mut hooks,
                    routine("MyDosRead"),
                    &args(&[cache, global("tags")], &[]),
                );
                assert_eq!(
                    vm.cpu().registers().a,
                    if count == 65 { 2 } else { 1 },
                    "{mode:?}/{crlf}/{count}"
                );
                assert_eq!(vm.bus().ram().read(cache + 1283), u8::from(count <= 64));
                if count <= 64 {
                    assert_eq!(vm.bus().ram().read(cache + 1282), count as u8);
                    assert_eq!(vm.bus().ram().read_word(global("tags") + 4), count as u16);
                    for (ordinal, file) in expected.iter().enumerate() {
                        let slot = vm.bus().ram().read(cache + 1285 + ordinal as u16);
                        let entry = cache + slot as u16 * 20;
                        assert_eq!(
                            machine::bytes(&vm, entry + 6, 11),
                            format!("{:<8}{:<3}", file.name, file.extension).into_bytes(),
                            "native entry before rendering: {mode:?}/{count}/{ordinal}, flags {}, order {:?}",
                            vm.bus().ram().read(entry + 4),
                            machine::bytes(&vm, cache + 1285, count)
                        );
                        assert_eq!(vm.bus().ram().read_word(entry), ordinal as u16);
                        vm = machine::call(
                            vm,
                            &mut hooks,
                            routine("MyDosName"),
                            &args(&[entry, name, 15], &[]),
                        );
                        assert_eq!(vm.cpu().registers().a, 1);
                        assert_eq!(machine::counted(&vm, name), file.filename());
                        for tagged in [false, true] {
                            let mut arguments = args(&[entry], &[u8::from(tagged)]);
                            arguments.extend(row.to_le_bytes());
                            vm = machine::call(vm, &mut hooks, routine("MyDosRender"), &arguments);
                            assert_eq!(machine::bytes(&vm, row, 18), file.row(tagged), "{file:?}");
                        }
                        let before = machine::bytes(&vm, name, 15);
                        vm = machine::call(
                            vm,
                            &mut hooks,
                            routine("MyDosName"),
                            &args(&[entry, name, 2], &[]),
                        );
                        assert_eq!(vm.cpu().registers().a, 2);
                        assert_eq!(
                            machine::bytes(&vm, name, 15),
                            before,
                            "no partial success on short output"
                        );
                    }
                    vm = machine::call(
                        vm,
                        &mut hooks,
                        routine("MyDosRenderSummary"),
                        &args(&[cache, row], &[]),
                    );
                    let mut expected_summary = b"999 FREE SECTORS"
                        .iter()
                        .copied()
                        .map(directory::screen)
                        .collect::<Vec<_>>();
                    expected_summary.resize(18, 0);
                    assert_eq!(machine::bytes(&vm, row, 18), expected_summary);
                }
                for (start, length) in [(cache - 1, 1369), (name - 1, 17), (row - 1, 20)] {
                    assert_eq!(vm.bus().ram().read(start), 0xA5);
                    assert_eq!(vm.bus().ram().read(start + length - 1), 0xA5);
                }
                for bad_input in [
                    vec![],
                    vec![vec![b'X'; 19]],
                    vec![directory::files(1)[0].input()],
                ] {
                    hooks.input = bad_input;
                    vm = machine::call(
                        vm,
                        &mut hooks,
                        routine("MyDosRead"),
                        &args(&[cache, global("tags")], &[]),
                    );
                    assert_eq!(vm.cpu().registers().a, 3);
                    assert_eq!(
                        vm.bus().ram().read(cache + 1283),
                        0,
                        "failed reads invalidate old entries"
                    );
                }
            }
        }
    }
}
