use super::*;

fn parameter(output: &CodegenOutput, routine: &str, name: &str) -> u16 {
    output
        .map
        .storage_symbols
        .iter()
        .find(|s| {
            s.scope == actionc::codegen::CodegenSymbolScope::Routine(routine.to_owned())
                && s.name == name
        })
        .unwrap()
        .address
}

struct ChangeSource {
    first: u16,
    second_high: u16,
    fired: bool,
    observe: Observe,
}
impl VmRunHooks for ChangeSource {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let pc = vm.cpu().registers().pc;
        if !self.fired
            && vm.bus().ram().read(pc) == 0xAD
            && vm.bus().ram().read_word(pc + 1) == self.second_high
        {
            vm.bus_mut().ram_mut().write_word(self.first, 0xFFFF);
            self.fired = true;
        }
        self.observe.before_step(vm)
    }
}

#[test]
fn argument_staging_retains_the_capture_when_its_source_changes_during_setup() {
    let text =
        include_str!("../../../../fixtures/runtime/argument_staging.act").replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for optimized in [false, true] {
            for enabled in [false, true] {
                for crlf in [false, true] {
                    let text = if crlf {
                        text.replace('\n', "\r\n")
                    } else {
                        text.clone()
                    };
                    let output = compile(&text, optimized, runtime, 0x3000, enabled);
                    let entry = |name: &str| {
                        output
                            .routine_addresses
                            .iter()
                            .find(|r| r.name == name)
                            .unwrap()
                            .address
                    };
                    let first = parameter(&output, "ForwardAdd", "first");
                    let second_high = parameter(&output, "ForwardAdd", "second") + 1;
                    let mut hooks = ChangeSource {
                        first,
                        second_high,
                        fired: false,
                        observe: Observe {
                            callees: [(entry("Capture"), 5)].into(),
                            arguments: Vec::new(),
                            states: Vec::new(),
                        },
                    };
                    let (mut vm, profile) = support::vm_for(runtime);
                    vm.load_atari_object_for_execution(
                        profile,
                        &actionc::codegen::format_load_file(&output),
                    )
                    .unwrap();
                    vm = machine::call(
                        vm,
                        &mut hooks,
                        entry("ForwardAdd"),
                        &[0xFF, 0, 0x34, 0x92, 0x80],
                    );
                    assert!(hooks.fired);
                    assert_eq!(hooks.observe.arguments, vec![vec![0, 1, 0x34, 0x92, 0x80]]);
                    assert_eq!(machine::bytes(&vm, 0x680, 5), vec![0, 1, 0x34, 0x92, 0x80]);
                    assert_eq!(vm.bus().ram().read_word(first), 0xFFFF);
                }
            }
        }
    }
}

struct ReadAlias {
    range: (u16, u16),
    address: u16,
    values: Vec<u8>,
}
impl VmRunHooks for ReadAlias {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let r = vm.cpu().registers();
        let ram = vm.bus().ram();
        if (self.range.0..self.range.1).contains(&r.pc) && ram.read(r.pc) == 0xB1 {
            let pair = ram.read(r.pc + 1);
            let base = u16::from_le_bytes([
                ram.read(u16::from(pair)),
                ram.read(u16::from(pair.wrapping_add(1))),
            ]);
            if base.wrapping_add(u16::from(r.y)) == self.address {
                self.values.push(ram.read(self.address));
            }
        }
        Ok(())
    }
}

#[test]
fn argument_staging_keeps_memory_observable_through_a_callee_pointer() {
    let source = "MODULE\nCARD observedFirst=$0680,observedSecond=$0682\nBYTE observedTag=$0684,observedByte=$0685\n\
        PROC Capture(CARD first,second BYTE tag)\nBYTE POINTER p\n\
          observedFirst=first observedSecond=second observedTag=tag\np=second observedByte=p^\nRETURN\n\
        PROC Forward(CARD first,second BYTE tag)\nCapture(first+1,second,tag)\nRETURN\nPROC Main()\nRETURN\n";
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for optimized in [false, true] {
            for crlf in [false, true] {
                let source = if crlf {
                    source.replace('\n', "\r\n")
                } else {
                    source.to_owned()
                };
                let mut expected = None;
                for enabled in [false, true] {
                    let output = compile(&source, optimized, runtime, 0x3000, enabled);
                    let entry = output
                        .routine_addresses
                        .iter()
                        .find(|r| r.name == "Forward")
                        .unwrap()
                        .address;
                    let capture = output
                        .map
                        .routine_ranges
                        .iter()
                        .find(|r| r.name == "Capture")
                        .unwrap();
                    let own_first = parameter(&output, "Forward", "first");
                    let (mut vm, profile) = support::vm_for(runtime);
                    vm.load_atari_object_for_execution(
                        profile,
                        &actionc::codegen::format_load_file(&output),
                    )
                    .unwrap();
                    let mut results = Vec::new();
                    for address in [
                        0xE0,
                        0xE1,
                        0xA0,
                        0xA1,
                        0xA2,
                        0xA3,
                        0xA4,
                        own_first,
                        own_first + 1,
                        0x7000,
                        0x70FF,
                    ] {
                        vm.bus_mut().ram_mut().write(address, 0x72);
                        let mut args = vec![0xFF, 0];
                        args.extend(address.to_le_bytes());
                        args.push(128);
                        let mut hooks = ReadAlias {
                            range: (capture.start, capture.end),
                            address,
                            values: Vec::new(),
                        };
                        vm = machine::call(vm, &mut hooks, entry, &args);
                        assert_eq!(
                            hooks.values.len(),
                            1,
                            "{runtime:?}/{optimized}/{enabled}/{address:04X}"
                        );
                        assert_eq!(
                            machine::bytes(&vm, 0x680, 5),
                            vec![0, 1, address as u8, (address >> 8) as u8, 128]
                        );
                        assert_eq!(vm.bus().ram().read(0x685), hooks.values[0]);
                        results.push((hooks.values[0], vm.bus().ram().read(0x685)));
                    }
                    if let Some(reference) = &expected {
                        assert_eq!(&results, reference);
                    } else {
                        expected = Some(results);
                    }
                }
            }
        }
    }
}
