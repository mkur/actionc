//! Classic Amiga library adapters. No source names or SemIR are consumed here.
use super::machine::{Ea, Instruction, Width};
use crate::runtime_fault::RuntimeFault;
use std::collections::BTreeMap;

#[path = "amiga_program.rs"]
mod program;
pub use program::materialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConsoleService {
    Put,
    PutE,
    Print,
    PrintE,
    PrintB,
    PrintBE,
    PrintC,
    PrintCE,
    PrintI,
    PrintIE,
}

/// Separate identity space for compiler-owned entry points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlatformRoutineId {
    Startup,
    Cleanup,
    Finish,
    Failure,
    WriteSpan,
    Fault(RuntimeFault),
    Console(ConsoleService),
    Library(LibraryCall),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlatformDataId {
    State,
    LibraryName,
    FaultMessage(RuntimeFault),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformData {
    pub bytes: Vec<u8>,
    pub zero_fill: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Platform {
    pub entry: Option<PlatformRoutineId>,
    pub routines: BTreeMap<PlatformRoutineId, super::machine::MachineBlockId>,
    pub data: BTreeMap<PlatformDataId, PlatformData>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LibraryCall {
    OpenLibrary,
    CloseLibrary,
    Output,
    Write,
}

impl LibraryCall {
    // NDK 3.2 rev4: Include_I/lvo/{exec,dos}_lib.i and FD/{exec,dos}_lib.fd.
    // https://developer.amigaos3.net/sites/default/files/downloads/2023-12/NDK3.2.lha
    // Archive SHA-256: 96cabd4ad683dced632e147bf86dee0f50dcb1254386216c25c362916a6409bb
    // Classic register contracts:
    // https://developer.amigaos3.net/autodocs/exec.library/OpenLibrary.html
    // https://developer.amigaos3.net/autodocs/exec.library/CloseLibrary.html
    // https://developer.amigaos3.net/autodocs/dos.library/Output.html
    // https://developer.amigaos3.net/autodocs/dos.library/Write.html
    pub const fn vector(self) -> i16 {
        match self {
            Self::OpenLibrary => -552,
            Self::CloseLibrary => -414,
            Self::Output => -60,
            Self::Write => -48,
        }
    }

    fn arguments(self) -> &'static [(Ea, bool)] {
        match self {
            Self::OpenLibrary => &[(Ea::A(1), true), (Ea::D(0), false)],
            Self::CloseLibrary => &[(Ea::A(1), true)],
            Self::Output => &[],
            Self::Write => &[(Ea::D(1), false), (Ea::D(2), true), (Ea::D(3), false)],
        }
    }

    pub fn argument_count(self) -> usize {
        self.arguments().len()
    }
}

/// Width at the Action! stack boundary, before conversion to an OS longword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Argument {
    pub width: Width,
    pub signed: bool,
}
impl Argument {
    pub const LONG: Self = Self {
        width: Width::Long,
        signed: false,
    };
}

/// The first incoming stack slot is always a full-width library base. Remaining
/// slots follow the Action! ABI (including a BYTE in the first byte of its slot).
/// OS pointers require a long slot. Scalars can be explicitly narrowed at this
/// private adapter boundary; the adapter extends them before the library call.
pub fn library_adapter(
    call: LibraryCall,
    arguments: &[Argument],
) -> Result<Vec<Instruction>, String> {
    if arguments.len() != call.argument_count() {
        return Err("Amiga adapter argument count does not match library signature".into());
    }
    let mov = |width, source, destination| Instruction::Move {
        width,
        source,
        destination,
    };
    let mut code = vec![
        Instruction::Link {
            register: 6,
            displacement: -12,
        },
        mov(Width::Long, Ea::D(2), Ea::Displacement(7, 0)),
        mov(Width::Long, Ea::D(3), Ea::Displacement(7, 4)),
        mov(Width::Long, Ea::A(6), Ea::Displacement(7, 8)),
    ];
    let mut offset = 12;
    for (&arg, &(register, pointer)) in arguments.iter().zip(call.arguments()) {
        if pointer && (arg.width != Width::Long || arg.signed) {
            return Err("Amiga adapter pointer requires an unsigned full-width slot".into());
        }
        if arg.width != Width::Long {
            let Ea::D(register) = register else {
                return Err("narrow Amiga adapter argument requires a data register".into());
            };
            code.push(Instruction::MoveQuick { value: 0, register });
        }
        code.push(mov(arg.width, Ea::Displacement(6, offset), register));
        if arg.signed && arg.width != Width::Long {
            let Ea::D(register) = register else {
                unreachable!()
            };
            if arg.width == Width::Byte {
                code.push(Instruction::Extend {
                    to: Width::Word,
                    register,
                });
            }
            code.push(Instruction::Extend {
                to: Width::Long,
                register,
            });
        }
        offset += arg.width.bytes().max(2) as i16;
    }
    // Capture everything before replacing the Action! frame pointer with A6's
    // foreign meaning. The OS call preserves SP, allowing restoration via SP.
    code.extend([
        mov(Width::Long, Ea::Displacement(6, 8), Ea::A(6)),
        Instruction::Jsr(Ea::Displacement(6, call.vector())),
        mov(Width::Long, Ea::Displacement(7, 8), Ea::A(6)),
        mov(Width::Long, Ea::Displacement(7, 0), Ea::D(2)),
        mov(Width::Long, Ea::Displacement(7, 4), Ea::D(3)),
    ]);
    if call == LibraryCall::OpenLibrary {
        code.push(mov(Width::Long, Ea::D(0), Ea::A(0)));
    }
    code.extend([Instruction::Unlink(6), Instruction::Rts]);
    Ok(code)
}
