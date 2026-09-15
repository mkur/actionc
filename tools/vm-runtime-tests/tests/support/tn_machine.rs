//! Common execution setup for the intact TN programs and their Action! fixtures.
use super::root;
use actionc::compiler::CompiledProgram;
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunRequest,
    StopReason, VmRunHooks, VmRunner,
};

pub const ENTRY: u16 = 0x0600;
pub const RETURN: u16 = 0x0620;
pub const STOP: u16 = 0x0610;

pub fn load(compiled: &CompiledProgram) -> CompilerVm {
    let mut vm = CompilerVm::default();
    for (kind, name, base) in [
        (ImageKind::Cartridge, "action.rom", DEFAULT_CART_BASE),
        (ImageKind::Rom, "altirraos-xl.rom", OS_ROM_BASE),
    ] {
        vm.load_image_bytes(
            kind,
            name,
            base,
            std::fs::read(root().join("roms").join(name)).unwrap(),
        )
        .unwrap();
    }
    let loaded = vm
        .load_atari_object_for_execution(ExecutionProfile::CartridgeObject, compiled.object_bytes())
        .unwrap();
    assert!(
        loaded
            .segments
            .iter()
            .all(|s| s.end < ENTRY || s.start > RETURN)
    );
    vm.bus_mut().ram_mut().write(RETURN, 0x60);
    vm
}

pub fn call<H: VmRunHooks<Error = String>>(
    mut vm: CompilerVm,
    hooks: &mut H,
    address: u16,
    arguments: &[u8],
) -> CompilerVm {
    let byte = |i| arguments.get(i).copied().unwrap_or(0);
    let ram = vm.bus_mut().ram_mut();
    ram.map(
        ENTRY,
        &[
            0xA9,
            byte(0),
            0xA2,
            byte(1),
            0xA0,
            byte(2),
            0x20,
            address as u8,
            (address >> 8) as u8,
            0x4C,
            STOP as u8,
            (STOP >> 8) as u8,
        ],
    )
    .unwrap();
    if arguments.len() > 3 {
        ram.map(0xA3, &arguments[3..]).unwrap();
    }
    ram.write(STOP, 0xEA);
    vm.set_pc(ENTRY);
    let sp = vm.cpu().registers().sp;
    let result = VmRunner::new(vm)
        .run_with_hooks(
            RunRequest {
                max_steps: 2_000_000,
                stop_after_pc: Some(STOP),
                history_len: 8,
            },
            hooks,
        )
        .unwrap();
    assert_eq!(
        result.stop_reason(),
        StopReason::PcReached { pc: STOP },
        "{:?}",
        result.report
    );
    assert_eq!(result.report.registers.sp, sp, "TN call leaked stack space");
    result.into_vm()
}

pub fn bytes(vm: &CompilerVm, address: u16, count: usize) -> Vec<u8> {
    (0..count)
        .map(|i| vm.bus().ram().read(address + i as u16))
        .collect()
}

pub fn counted(vm: &CompilerVm, address: u16) -> Vec<u8> {
    bytes(vm, address + 1, vm.bus().ram().read(address) as usize)
}
