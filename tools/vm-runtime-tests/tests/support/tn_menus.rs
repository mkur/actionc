//! Exercise PopUp and its actual lookup/bar routines; stub input and drawing.
use super::{machine, routine_address};
use actionc::compiler::CompiledProgram;
use actionc_vm::{CompilerVm, VmRunHooks};
use std::collections::{BTreeMap, VecDeque};

const KEY_RESULT: u16 = 0x0630;
const ARENA: u16 = 0x2000;
type Input = (u8, Option<u8>);

struct MenuHooks {
    entries: BTreeMap<u16, &'static str>,
    input: VecDeque<Input>,
    limit: u8,
    lookups: Vec<u8>,
    bars: Vec<u8>,
}

impl VmRunHooks for MenuHooks {
    type Error = String;

    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let Some(&name) = self.entries.get(&vm.cpu().registers().pc) else {
            return Ok(());
        };
        match name {
            "FindItem" => self.lookups.push(vm.cpu().registers().y),
            "MoveMenuBar" => self.bars.push(vm.cpu().registers().y),
            "Range" => {
                assert_eq!(vm.cpu().registers().y, self.limit);
                let (key, selected) = self.input.pop_front().ok_or("unexpected menu input")?;
                let ram = vm.bus_mut().ram_mut();
                // Range returns a key and can update its selection parameter.
                if let Some(selected) = selected {
                    let address = ram.read_word(0xA3);
                    ram.write(address, selected);
                }
                ram.map(KEY_RESULT, &[0xA9, key, 0x85, 0xA0, 0x60])?;
                vm.set_pc(KEY_RESULT);
            }
            _ => vm.set_pc(machine::RETURN),
        }
        Ok(())
    }
}

pub fn check(compiled: &CompiledProgram) {
    let listing = compiled.source_listing();
    let routine = |name| routine_address(&listing, name);
    let mut hooks = MenuHooks {
        entries: [
            "FindItem",
            "MoveMenuBar",
            "Range",
            "Print",
            "Relpos",
            "Block",
        ]
        .into_iter()
        .map(|name| (routine(name), name))
        .collect(),
        input: VecDeque::new(),
        limit: 0,
        lookups: vec![],
        bars: vec![],
    };
    let mut vm = machine::load(compiled);
    // Keep PopUp's real Push/SavePos/RestorePos/Pull path active.
    vm.bus_mut()
        .ram_mut()
        .map(0xE8, &ARENA.to_le_bytes())
        .unwrap();
    let position = [3, 5, 12, 7];
    vm.bus_mut().ram_mut().map(0x5A, &position).unwrap();

    struct Case {
        menu: &'static str,
        initial: u8,
        input: &'static [Input],
        result: u8,
        bars: &'static [u8],
    }
    // Reuse the VM across calls to expose stale persistent-local values.
    let cases = [
        Case {
            menu: "density",
            initial: 4,
            input: &[(b'S', None)],
            result: b'S',
            bars: &[4, 4, 0],
        },
        Case {
            menu: "density",
            initial: 0,
            input: &[(b'7', None)],
            result: b'7',
            bars: &[0, 0, 4],
        },
        Case {
            menu: "density",
            initial: 2,
            input: &[(b'D', None)],
            result: b'D',
            bars: &[2],
        },
        Case {
            menu: "density",
            initial: 3,
            input: &[(155, None)],
            result: b'3',
            bars: &[3],
        },
        Case {
            menu: "density",
            initial: 1,
            input: &[(27, None)],
            result: 27,
            bars: &[1],
        },
        Case {
            menu: "density",
            initial: 0,
            input: &[(b'?', None), (b'*', Some(1)), (155, None)],
            result: b'M',
            bars: &[0, 0, 1],
        },
        Case {
            menu: "density",
            initial: 4,
            input: &[(0x1E, Some(3)), (b'?', None), (b'D', None)],
            result: b'D',
            bars: &[4, 4, 3, 3, 2],
        },
        Case {
            menu: "delcancel",
            initial: 1,
            input: &[(b'D', None)],
            result: b'D',
            bars: &[1, 1, 0],
        },
        Case {
            menu: "delcancel",
            initial: 1,
            input: &[(b'?', None), (155, None)],
            result: b'C',
            bars: &[1],
        },
    ];
    for case in cases {
        hooks.input = case.input.iter().copied().collect();
        hooks.limit = if case.menu == "density" { 4 } else { 1 };
        hooks.lookups.clear();
        hooks.bars.clear();
        let menu = routine(case.menu);
        vm = machine::call(
            vm,
            &mut hooks,
            routine("PopUp"),
            &[menu as u8, (menu >> 8) as u8, case.initial, 2],
        );
        assert_eq!(
            vm.bus().ram().read(0xA0),
            case.result,
            "{} {:?}",
            case.menu,
            case.input
        );
        assert!(hooks.input.is_empty(), "menu exited before consuming input");
        assert_eq!(
            hooks.lookups,
            case.input.iter().map(|&(key, _)| key).collect::<Vec<_>>(),
            "one fresh lookup per input key"
        );
        assert_eq!(hooks.bars, case.bars, "menu highlight transitions");
        assert_eq!(
            vm.bus().ram().read_word(0xE8),
            ARENA,
            "menu leaked arena storage"
        );
        assert_eq!(
            machine::bytes(&vm, 0x5A, 4),
            position,
            "menu position restoration"
        );
    }
}
