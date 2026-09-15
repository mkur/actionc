//! Execute the actual source-owned location routines, with guarded storage.
use super::{machine, routine_address};
use actionc::compiler::CompiledProgram;
use actionc_vm::{CompilerVm, VmRunHooks};
struct NoIo;
impl VmRunHooks for NoIo {
    type Error = String;
    fn before_step(&mut self, _: &mut CompilerVm) -> Result<(), String> { Ok(()) }
}
fn words(values: &[u16]) -> Vec<u8> { values.iter().flat_map(|v| v.to_le_bytes()).collect() }
pub fn check(compiled: &CompiledProgram) {
    let listing = compiled.source_listing();
    let routine = |s| routine_address(&listing, s);
    let mut vm = machine::load(compiled);
    let mut hooks = NoIo;
    let location = 0x0801;
    let name = 0x0900;
    vm.bus_mut().ram_mut().map(location - 1, &[0xA5; 51]).unwrap();
    let mut root_args = words(&[location]); root_args.push(2);
    vm = machine::call(vm, &mut hooks, routine("MyDosRoot"), &root_args);
    assert_eq!(vm.bus().ram().read_word(location), 0x169);
    assert_eq!(vm.bus().ram().read(location + 11), 2);
    for (level, stem) in ["ABCDEFGH", "IJKLMNOP", "QRSTUVWX", "YZ012345"].iter().enumerate() {
        let filename = format!("D:{stem}.");
        vm.bus_mut().ram_mut().write(name, filename.len() as u8);
        vm.bus_mut().ram_mut().map(name + 1, filename.as_bytes()).unwrap();
        vm = machine::call(vm, &mut hooks, routine("MyDosPush"), &words(&[location, name, 0x169 + level as u16, 0x16A + level as u16]));
        assert_eq!(vm.cpu().registers().a, 1);
        assert_eq!(vm.bus().ram().read(location + 10), level as u8 + 1);
        let slot = location + 13 + level as u16 * 9;
        assert_eq!(machine::counted(&vm, slot), stem.as_bytes());
        assert_eq!(vm.bus().ram().read(slot + 9), 0xA5, "dot must not overwrite next name or guard");
        let mut path_args = words(&[location]); path_args.push(level as u8 + 1);
        vm = machine::call(vm, &mut hooks, routine("MyDosPathName"), &path_args);
        assert_eq!(vm.bus().ram().read_word(0xA0), slot);
    }
    let before = machine::bytes(&vm, location, 49);
    vm = machine::call(vm, &mut hooks, routine("MyDosPush"), &words(&[location, name, 0x7777, 0x8888]));
    assert_eq!(vm.cpu().registers().a, 2);
    assert_eq!(machine::bytes(&vm, location, 49), before);
    for level in (0..4u8).rev() {
        vm = machine::call(vm, &mut hooks, routine("MyDosParent"), &words(&[location]));
        assert_eq!(vm.cpu().registers().a, 1);
        assert_eq!(vm.bus().ram().read(location + 10), level);
        assert_eq!(vm.bus().ram().read_word(location), 0x169 + u16::from(level));
    }
    vm = machine::call(vm, &mut hooks, routine("MyDosParent"), &words(&[location]));
    assert_eq!(vm.cpu().registers().a, 0);
    for invalid in ["D:.", "D:ABCDEFGHI.", "D:ABCDEFGH", "Q:ABC."] {
        vm.bus_mut().ram_mut().write(name, invalid.len() as u8);
        vm.bus_mut().ram_mut().map(name + 1, invalid.as_bytes()).unwrap();
        let before = machine::bytes(&vm, location, 49);
        vm = machine::call(vm, &mut hooks, routine("MyDosPush"), &words(&[location, name, 0x7777, 0x8888]));
        assert_eq!(vm.cpu().registers().a, 3);
        assert_eq!(machine::bytes(&vm, location, 49), before);
    }
    for guard in [location - 1, location + 49] { assert_eq!(vm.bus().ram().read(guard), 0xA5); }
}
