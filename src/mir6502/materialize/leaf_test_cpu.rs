//! Leaf fixtures share the bounded emitted-code executor with codegen tests.
#[path = "../../codegen/test_cpu.rs"]
mod cpu;

pub(super) use cpu::run_memory;

pub(super) fn run(bytes: &[u8], origin: u16, entry: usize, input: u8) -> (u8, Vec<u8>) {
    let mut memory = [0u8; 65536];
    memory[usize::from(origin)..usize::from(origin) + bytes.len()].copy_from_slice(bytes);
    memory[0x600] = input;
    for index in 0..=255 {
        memory[0x700 + index] = (index as u8) ^ 0xa5;
    }
    let returns = run_memory(&mut memory, usize::from(origin) + entry);
    (memory[0x601], returns)
}
