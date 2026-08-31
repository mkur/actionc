use super::diagnostics::MirDiagnostic;
use super::ir::{
    MirBlock, MirBlockId, MirFrame, MirMachineBlock, MirMachineBlockId, MirMachineItem, MirOp,
    MirProgram, MirRegisterSet, MirRoutine, MirRoutineAbi, MirRuntimeHelper,
    MirRuntimeHelperTarget, MirTerminator, RoutineId,
};
use crate::runtime::Runtime;

const GENERATED_BYTE_MULTIPLY_BYTES: [u8; 42] = [
    0x85, 0x82, // STA $82: multiplicand low
    0x86, 0x84, // STX $84: multiplier
    0xA9, 0x00, // LDA #0
    0x85, 0x83, // multiplicand high
    0x85, 0x86, // result low
    0x85, 0x87, // result high
    0xA0, 0x08, // LDY #8
    0x46, 0x84, // loop: LSR $84
    0x90, 0x0D, // BCC no_add
    0x18, // CLC
    0xA5, 0x86, // LDA $86
    0x65, 0x82, // ADC $82
    0x85, 0x86, // STA $86
    0xA5, 0x87, // LDA $87
    0x65, 0x83, // ADC $83
    0x85, 0x87, // STA $87
    0x06, 0x82, // no_add: ASL $82
    0x26, 0x83, // ROL $83
    0x88, // DEY
    0xD0, 0xE8, // BNE loop
    0xA5, 0x86, // LDA $86
    0xA6, 0x87, // LDX $87
];

pub(super) fn resolve_helpers(
    program: &mut MirProgram,
    runtime: Runtime,
) -> Result<(), Vec<MirDiagnostic>> {
    bind_generated_byte_multiply(program);
    program
        .runtime_helpers
        .sort_by_key(|declaration| declaration.helper);

    if runtime == Runtime::Standalone {
        return super::standalone::link_helpers(program);
    }

    for declaration in &mut program.runtime_helpers {
        if !matches!(declaration.target, MirRuntimeHelperTarget::Deferred) {
            continue;
        }
        declaration.target =
            MirRuntimeHelperTarget::KnownAbsolute(cartridge_address(declaration.helper));
    }
    Ok(())
}

pub(super) const fn helper_name(helper: MirRuntimeHelper) -> &'static str {
    match helper {
        MirRuntimeHelper::MulByte => "MultB",
        MirRuntimeHelper::Mul => "MultI",
        MirRuntimeHelper::Div => "DivI",
        MirRuntimeHelper::Mod => "RemI",
        MirRuntimeHelper::Lsh => "LShift",
        MirRuntimeHelper::Rsh => "RShift",
        MirRuntimeHelper::SArgs => "SArgs",
    }
}

fn cartridge_address(helper: MirRuntimeHelper) -> u16 {
    use crate::codegen::runtime_helper;

    match helper {
        MirRuntimeHelper::MulByte => {
            unreachable!("compiler-owned MultB is bound before cartridge resolution")
        }
        MirRuntimeHelper::Mul => runtime_helper::CARTRIDGE_MUL.address(),
        MirRuntimeHelper::Div => runtime_helper::CARTRIDGE_DIV.address(),
        MirRuntimeHelper::Mod => runtime_helper::CARTRIDGE_MOD.address(),
        MirRuntimeHelper::Lsh => runtime_helper::CARTRIDGE_LSH.address(),
        MirRuntimeHelper::Rsh => runtime_helper::CARTRIDGE_RSH.address(),
        MirRuntimeHelper::SArgs => runtime_helper::CARTRIDGE_SARGS.address(),
    }
}

fn bind_generated_byte_multiply(program: &mut MirProgram) {
    let Some(declaration_index) = program.runtime_helpers.iter().position(|declaration| {
        declaration.helper == MirRuntimeHelper::MulByte
            && matches!(declaration.target, MirRuntimeHelperTarget::Deferred)
    }) else {
        return;
    };

    let routine_id = RoutineId(
        program
            .routines
            .iter()
            .map(|routine| routine.id.0)
            .max()
            .map_or(0, |id| id.wrapping_add(1)),
    );
    let machine_id = MirMachineBlockId(
        program
            .machine_blocks
            .iter()
            .map(|machine| machine.id.0)
            .max()
            .map_or(0, |id| id.wrapping_add(1)),
    );
    let mut effects = super::materialize::helper_effects(&MirRuntimeHelper::MulByte);
    effects.reads = MirRegisterSet {
        a: true,
        x: true,
        ..MirRegisterSet::default()
    };

    // Input is A:X, output is the complete unsigned product in A:X. The
    // generated helper is target-owned and therefore works with either the
    // cartridge or standalone runtime without depending on a private ROM
    // entry point.
    program.machine_blocks.push(MirMachineBlock {
        id: machine_id,
        items: GENERATED_BYTE_MULTIPLY_BYTES
            .into_iter()
            .map(MirMachineItem::Byte)
            .collect(),
    });
    program.routines.push(MirRoutine {
        id: routine_id,
        name: "ACTION.RUNTIME.ACTIONC::MultB".to_string(),
        abi: MirRoutineAbi::ActionObservable,
        frame: MirFrame::default(),
        temps: Vec::new(),
        blocks: vec![MirBlock {
            id: MirBlockId(0),
            label: "entry".to_string(),
            params: Vec::new(),
            ops: vec![MirOp::MachineBlock {
                id: machine_id,
                effects: effects.clone(),
            }],
            terminator: MirTerminator::Return,
        }],
        effects,
    });
    program.runtime_helpers[declaration_index].target = MirRuntimeHelperTarget::Routine(routine_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::MirRuntimeHelperDecl;

    #[test]
    fn cart_resolution_preserves_established_addresses() {
        let mut program = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: Vec::new(),
            machine_blocks: Vec::new(),
            runtime_helpers: [
                MirRuntimeHelper::Mul,
                MirRuntimeHelper::Div,
                MirRuntimeHelper::Mod,
                MirRuntimeHelper::Lsh,
                MirRuntimeHelper::Rsh,
                MirRuntimeHelper::SArgs,
            ]
            .into_iter()
            .map(|helper| MirRuntimeHelperDecl {
                helper,
                target: MirRuntimeHelperTarget::Deferred,
                abi: crate::mir6502::materialize::helper_abi(),
                effects: crate::mir6502::materialize::helper_effects(&helper),
            })
            .collect(),
        };

        resolve_helpers(&mut program, Runtime::ActionCart).unwrap();

        let addresses = program
            .runtime_helpers
            .iter()
            .map(|decl| match decl.target {
                MirRuntimeHelperTarget::KnownAbsolute(address) => address,
                _ => panic!("cart helper was not resolved to an absolute address"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            addresses,
            vec![0xA000, 0xA090, 0xA0DE, 0xB5C0, 0xA0E6, 0xA0F5]
        );
    }

    #[test]
    fn byte_multiply_binds_a_target_owned_helper_for_both_runtimes() {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let mut program = MirProgram {
                statics: Vec::new(),
                globals: Vec::new(),
                routines: Vec::new(),
                machine_blocks: Vec::new(),
                runtime_helpers: vec![MirRuntimeHelperDecl {
                    helper: MirRuntimeHelper::MulByte,
                    target: MirRuntimeHelperTarget::Deferred,
                    abi: crate::mir6502::materialize::helper_abi(),
                    effects: crate::mir6502::materialize::helper_effects(
                        &MirRuntimeHelper::MulByte,
                    ),
                }],
            };

            resolve_helpers(&mut program, runtime).expect("resolve byte multiply helper");

            let MirRuntimeHelperTarget::Routine(target) = program.runtime_helpers[0].target else {
                panic!("byte multiply must bind to a generated routine")
            };
            assert_eq!(program.routines[0].id, target);
            assert_eq!(program.routines[0].name, "ACTION.RUNTIME.ACTIONC::MultB");
            assert_eq!(program.machine_blocks.len(), 1);
        }
    }

    #[test]
    fn generated_byte_multiply_returns_every_unsigned_product() {
        for left in 0..=u8::MAX {
            for right in 0..=u8::MAX {
                assert_eq!(
                    run_generated_byte_multiply(left, right),
                    u16::from(left) * u16::from(right),
                    "{left} * {right}"
                );
            }
        }
    }

    fn run_generated_byte_multiply(left: u8, right: u8) -> u16 {
        let mut code = GENERATED_BYTE_MULTIPLY_BYTES.to_vec();
        code.push(0x60);
        let mut memory = [0u8; 256];
        let (mut a, mut x, mut y) = (left, right, 0u8);
        let (mut carry, mut zero) = (false, false);
        let mut pc = 0usize;
        for _ in 0..256 {
            let opcode = code[pc];
            pc += 1;
            match opcode {
                0x06 => {
                    let address = code[pc] as usize;
                    pc += 1;
                    carry = memory[address] & 0x80 != 0;
                    memory[address] <<= 1;
                    zero = memory[address] == 0;
                }
                0x18 => carry = false,
                0x26 => {
                    let address = code[pc] as usize;
                    pc += 1;
                    let incoming = u8::from(carry);
                    carry = memory[address] & 0x80 != 0;
                    memory[address] = (memory[address] << 1) | incoming;
                    zero = memory[address] == 0;
                }
                0x46 => {
                    let address = code[pc] as usize;
                    pc += 1;
                    carry = memory[address] & 1 != 0;
                    memory[address] >>= 1;
                    zero = memory[address] == 0;
                }
                0x60 => return u16::from_le_bytes([a, x]),
                0x65 => {
                    let address = code[pc] as usize;
                    pc += 1;
                    let sum = u16::from(a) + u16::from(memory[address]) + u16::from(carry);
                    a = sum as u8;
                    carry = sum > u16::from(u8::MAX);
                    zero = a == 0;
                }
                0x85 => {
                    let address = code[pc] as usize;
                    pc += 1;
                    memory[address] = a;
                }
                0x86 => {
                    let address = code[pc] as usize;
                    pc += 1;
                    memory[address] = x;
                }
                0x88 => {
                    y = y.wrapping_sub(1);
                    zero = y == 0;
                }
                0x90 => {
                    let offset = code[pc] as i8;
                    pc += 1;
                    if !carry {
                        pc = pc.wrapping_add_signed(isize::from(offset));
                    }
                }
                0xA0 => {
                    y = code[pc];
                    pc += 1;
                    zero = y == 0;
                }
                0xA5 => {
                    let address = code[pc] as usize;
                    pc += 1;
                    a = memory[address];
                    zero = a == 0;
                }
                0xA6 => {
                    let address = code[pc] as usize;
                    pc += 1;
                    x = memory[address];
                    zero = x == 0;
                }
                0xA9 => {
                    a = code[pc];
                    pc += 1;
                    zero = a == 0;
                }
                0xD0 => {
                    let offset = code[pc] as i8;
                    pc += 1;
                    if !zero {
                        pc = pc.wrapping_add_signed(isize::from(offset));
                    }
                }
                _ => panic!("unsupported generated helper opcode ${opcode:02X}"),
            }
        }
        panic!("generated byte multiply did not return")
    }
}
