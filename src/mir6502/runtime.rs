use super::diagnostics::MirDiagnostic;
use super::ir::{
    MirBlock, MirBlockId, MirFrame, MirInlineAsmTarget, MirMachineBlock, MirMachineBlockId,
    MirMachineItem, MirOp, MirProgram, MirRegisterSet, MirRoutine, MirRoutineAbi, MirRuntimeHelper,
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
    for helper in [MirRuntimeHelper::Mul32, MirRuntimeHelper::Lsh32, MirRuntimeHelper::Rsh32] {
        if !program.runtime_helpers.iter().any(|decl| decl.helper == helper) { continue; }
        let body = match helper {
            MirRuntimeHelper::Mul32 => crate::integer6502::wide::multiply(),
            MirRuntimeHelper::Lsh32 => crate::integer6502::wide::shift_body(true),
            _ => crate::integer6502::wide::shift_body(false),
        };
        bind_generated_helper(program, helper, body.into_iter().map(MirMachineItem::Byte).collect());
    }
    let error_target = if program
        .runtime_helpers
        .iter()
        .any(|decl| requires_error(decl.helper))
    {
        Some(match runtime {
            Runtime::ActionCart => {
                MirInlineAsmTarget::Absolute(crate::integer6502::CARTRIDGE_ERROR)
            }
            Runtime::Standalone => {
                MirInlineAsmTarget::Routine(super::standalone::link_error(program)?)
            }
        })
    } else {
        None
    };
    for helper in [
        MirRuntimeHelper::InvalidVariant,
        MirRuntimeHelper::Div32,
        MirRuntimeHelper::Mod32,
        MirRuntimeHelper::UDiv32,
        MirRuntimeHelper::UMod32,
        MirRuntimeHelper::Div,
        MirRuntimeHelper::Mod,
        MirRuntimeHelper::UDiv,
        MirRuntimeHelper::UMod,
        MirRuntimeHelper::DivU8,
        MirRuntimeHelper::ModU8,
        MirRuntimeHelper::DivU16U8,
        MirRuntimeHelper::ModU16U8,
        MirRuntimeHelper::DivMod,
        MirRuntimeHelper::UDivMod,
    ] {
        if let Some(declaration) = program
            .runtime_helpers
            .iter()
            .find(|decl| decl.helper == helper)
        {
            if !matches!(declaration.target, MirRuntimeHelperTarget::Deferred) {
                return Err(vec![MirDiagnostic::routine(
                    "<runtime>",
                    "legacy division/remainder SET overrides cannot replace modern integer operators",
                )]);
            }
            let body = if helper == MirRuntimeHelper::InvalidVariant {
                crate::integer6502::fault_body()
            } else if helper.is_wide() {
                crate::integer6502::wide::division(
                    matches!(helper, MirRuntimeHelper::Div32 | MirRuntimeHelper::Mod32),
                    matches!(helper, MirRuntimeHelper::Mod32 | MirRuntimeHelper::UMod32),
                )
            } else if matches!(helper, MirRuntimeHelper::DivMod | MirRuntimeHelper::UDivMod) {
                crate::integer6502::divmod_body(helper == MirRuntimeHelper::DivMod)
            } else if matches!(
                helper,
                MirRuntimeHelper::DivU8
                    | MirRuntimeHelper::ModU8
                    | MirRuntimeHelper::DivU16U8
                    | MirRuntimeHelper::ModU16U8
            ) {
                crate::integer6502::narrow_division_body(
                    matches!(
                        helper,
                        MirRuntimeHelper::DivU16U8 | MirRuntimeHelper::ModU16U8
                    ),
                    matches!(helper, MirRuntimeHelper::ModU8 | MirRuntimeHelper::ModU16U8),
                )
            } else {
                crate::integer6502::division_body(
                    matches!(helper, MirRuntimeHelper::Div | MirRuntimeHelper::Mod),
                    matches!(helper, MirRuntimeHelper::Mod | MirRuntimeHelper::UMod),
                )
            };
            let mut items = Vec::new();
            for (offset, byte) in body.bytes.into_iter().enumerate() {
                if offset == body.error_operand {
                    items.push(MirMachineItem::Relocation {
                        kind: crate::asm6502::InlineAsmRelocationKind::Absolute16,
                        target: error_target.clone().expect("division error binding"),
                        addend: 0,
                        requires_zero_page: false,
                        span: crate::source::Span::new(0, 0),
                    });
                } else if offset != body.error_operand + 1 {
                    items.push(MirMachineItem::Byte(byte));
                }
            }
            bind_generated_helper(program, helper, items);
        }
    }
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

pub(super) fn requires_error(helper: MirRuntimeHelper) -> bool {
    helper == MirRuntimeHelper::InvalidVariant || is_division(helper)
}

pub(super) fn is_division(helper: MirRuntimeHelper) -> bool {
    matches!(
        helper,
        MirRuntimeHelper::Div32 | MirRuntimeHelper::Mod32 | MirRuntimeHelper::UDiv32 | MirRuntimeHelper::UMod32
            | MirRuntimeHelper::Div
            | MirRuntimeHelper::Mod
            | MirRuntimeHelper::UDiv
            | MirRuntimeHelper::UMod
            | MirRuntimeHelper::DivU8
            | MirRuntimeHelper::ModU8
            | MirRuntimeHelper::DivU16U8
            | MirRuntimeHelper::ModU16U8
            | MirRuntimeHelper::DivMod
            | MirRuntimeHelper::UDivMod
    )
}

pub(super) const fn helper_name(helper: MirRuntimeHelper) -> &'static str {
    match helper {
        MirRuntimeHelper::InvalidVariant => "InvalidVariant",
        MirRuntimeHelper::Mul32 => "Mult32",
        MirRuntimeHelper::Div32 => "DivI32",
        MirRuntimeHelper::Mod32 => "RemI32",
        MirRuntimeHelper::UDiv32 => "DivU32",
        MirRuntimeHelper::UMod32 => "RemU32",
        MirRuntimeHelper::Lsh32 => "LShift32",
        MirRuntimeHelper::Rsh32 => "RShift32",
        MirRuntimeHelper::MulByte => "MultB",
        MirRuntimeHelper::Mul => "MultI",
        MirRuntimeHelper::Div => "DivI",
        MirRuntimeHelper::Mod => "RemI",
        MirRuntimeHelper::UDiv => "DivU16",
        MirRuntimeHelper::UMod => "RemU16",
        MirRuntimeHelper::DivU8 => "DivU8",
        MirRuntimeHelper::ModU8 => "RemU8",
        MirRuntimeHelper::DivU16U8 => "DivU16U8",
        MirRuntimeHelper::ModU16U8 => "RemU16U8",
        MirRuntimeHelper::DivMod => "DivModI16",
        MirRuntimeHelper::UDivMod => "DivModU16",
        MirRuntimeHelper::Lsh => "LShift",
        MirRuntimeHelper::Rsh => "RShift",
        MirRuntimeHelper::SArgs => "SArgs",
    }
}

fn cartridge_address(helper: MirRuntimeHelper) -> u16 {
    use crate::codegen::runtime_helper;

    match helper {
        MirRuntimeHelper::Mul32 | MirRuntimeHelper::Div32 | MirRuntimeHelper::Mod32
        | MirRuntimeHelper::UDiv32 | MirRuntimeHelper::UMod32 | MirRuntimeHelper::Lsh32 | MirRuntimeHelper::Rsh32
        | MirRuntimeHelper::MulByte | MirRuntimeHelper::Div | MirRuntimeHelper::Mod
        | MirRuntimeHelper::UDiv | MirRuntimeHelper::UMod
        | MirRuntimeHelper::DivU8 | MirRuntimeHelper::ModU8 | MirRuntimeHelper::DivU16U8 | MirRuntimeHelper::ModU16U8
        | MirRuntimeHelper::DivMod | MirRuntimeHelper::UDivMod | MirRuntimeHelper::InvalidVariant => {
            unreachable!("compiler-owned arithmetic is bound before cartridge resolution")
        }
        MirRuntimeHelper::Mul => runtime_helper::CARTRIDGE_MUL.address(),
        MirRuntimeHelper::Lsh => runtime_helper::CARTRIDGE_LSH.address(),
        MirRuntimeHelper::Rsh => runtime_helper::CARTRIDGE_RSH.address(),
        MirRuntimeHelper::SArgs => runtime_helper::CARTRIDGE_SARGS.address(),
    }
}

fn bind_generated_byte_multiply(program: &mut MirProgram) {
    bind_generated_helper(
        program,
        MirRuntimeHelper::MulByte,
        GENERATED_BYTE_MULTIPLY_BYTES
            .into_iter()
            .map(MirMachineItem::Byte)
            .collect(),
    );
}

fn bind_generated_helper(
    program: &mut MirProgram,
    helper: MirRuntimeHelper,
    items: Vec<MirMachineItem>,
) {
    let Some(declaration_index) = program.runtime_helpers.iter().position(|declaration| {
        declaration.helper == helper
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
    let mut effects = super::materialize::helper_effects(&helper);
    effects.reads = MirRegisterSet {
        a: !helper.is_wide(),
        x: !helper.is_wide(),
        ..MirRegisterSet::default()
    };

    // The declaration owns each helper's input/output signature. The
    // generated helper is target-owned and therefore works with either the
    // cartridge or standalone runtime without depending on a private ROM
    // entry point.
    program.machine_blocks.push(MirMachineBlock {
        id: machine_id,
        items,
    });
    program.routines.push(MirRoutine {
        id: routine_id,
        name: format!("ACTION.RUNTIME.ACTIONC::{}", helper_name(helper)),
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
    fn cart_resolution_preserves_services_but_replaces_legacy_division() {
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
                additional_results: Vec::new(),
                helper,
                target: MirRuntimeHelperTarget::Deferred,
                abi: crate::mir6502::materialize::helper_abi_for(helper),
                effects: crate::mir6502::materialize::helper_effects(&helper),
            })
            .collect(),
        };

        resolve_helpers(&mut program, Runtime::ActionCart).unwrap();

        let addresses = program
            .runtime_helpers
            .iter()
            .filter_map(|decl| match decl.target {
                MirRuntimeHelperTarget::KnownAbsolute(address) => Some(address),
                MirRuntimeHelperTarget::Routine(_)
                    if matches!(decl.helper, MirRuntimeHelper::Div | MirRuntimeHelper::Mod) =>
                {
                    None
                }
                _ => panic!("cart service or owned helper not resolved"),
            })
            .collect::<Vec<_>>();
        assert_eq!(addresses, vec![0xA000, 0xB5C0, 0xA0E6, 0xA0F5]);
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
                    additional_results: Vec::new(),
                    helper: MirRuntimeHelper::MulByte,
                    target: MirRuntimeHelperTarget::Deferred,
                    abi: crate::mir6502::materialize::helper_abi_for(MirRuntimeHelper::MulByte),
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
