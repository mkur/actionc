#![allow(dead_code)]
use actionc::{
    compiler::native65816,
    includes::ModuleLoadOptions,
    mir65816::image::{Image, LinkOptions},
};
use actionc_vm::native65816::{self as cpu, Access, Inputs, Machine, Registers};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

pub struct Temp(pub PathBuf);
impl Temp {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "actionc-native65816-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn layout() -> LinkOptions {
    LinkOptions {
        code_origin: 0x018000,
        data_origin: 0x120000,
        read_only_origin: None,
        zero_fill_origin: None,
        stack_overflow: 0x048000,
        nmi_extra_stack: 0,
        imports: vec![],
    }
}
pub fn prepare(source: &str, optimize: bool) -> native65816::Prepared {
    let dir = Temp::new();
    let path = dir.0.join("test.act");
    std::fs::write(&path, source).unwrap();
    let mut modules = ModuleLoadOptions::default();
    modules
        .module_paths
        .push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime/65816"));
    native65816::prepare_file(&path, optimize, &modules).unwrap()
}
pub fn compile(source: &str, optimize: bool) -> Image {
    let compiled = prepare(source, optimize).compile(&layout()).unwrap();
    // Execution consumes the serialized artifact, not compiler IR or mutable
    // machine objects. The VM has no knowledge of Action! operations.
    Image::from_json(&compiled.image.to_json().unwrap()).unwrap()
}
pub struct Assembly {
    pub bytes: Vec<u8>,
    pub symbols: std::collections::BTreeMap<String, u32>,
}
pub fn assemble(source: &str, origin: u32) -> Vec<u8> {
    assemble_artifact(source, origin).bytes
}
pub fn assemble_artifact(source: &str, origin: u32) -> Assembly {
    let dir = Temp::new();
    let asm = dir.0.join("probe.s");
    let object = dir.0.join("probe.o");
    let binary = dir.0.join("probe.bin");
    let cfg = dir.0.join("probe.cfg");
    let labels = dir.0.join("probe.lbl");
    std::fs::write(
        &asm,
        format!(".setcpu \"65816\"\n.a16\n.i16\n.segment \"CODE\"\n{source}\n"),
    )
    .unwrap();
    std::fs::write(&cfg, format!("MEMORY {{ CODE: start = ${origin:06x}, size = $8000, file = %O; }}\nSEGMENTS {{ CODE: load = CODE, type = ro; }}\n")).unwrap();
    run(Command::new("ca65")
        .arg("-I")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/abi"))
        .arg(&asm)
        .arg("-o")
        .arg(&object));
    run(Command::new("ld65")
        .arg("-Ln")
        .arg(&labels)
        .arg("-C")
        .arg(&cfg)
        .arg(&object)
        .arg("-o")
        .arg(&binary));
    Assembly {
        bytes: std::fs::read(binary).unwrap(),
        symbols: std::fs::read_to_string(labels)
            .unwrap()
            .lines()
            .filter_map(|line| {
                let mut words = line.split_whitespace();
                if words.next() != Some("al") {
                    return None;
                }
                let address = u32::from_str_radix(words.next()?, 16).ok()?;
                Some((words.next()?.trim_start_matches('.').to_string(), address))
            })
            .collect(),
    }
}
fn run(command: &mut Command) {
    let result = command
        .output()
        .expect("install ca65 and ld65 for independent assembly qualification");
    assert!(
        result.status.success(),
        "{command:?}\n{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
pub fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
    .replace("\r\n", "\n")
}

#[derive(Clone)]
pub struct Bus {
    pub ram: Vec<u8>,
    regions: Vec<(u32, u32, bool)>,
    pub writes: Vec<(u32, u8)>,
    pub reads: Vec<u32>,
    pub watched: std::collections::BTreeSet<u32>,
    pub trace: Vec<(u64, u32, Access)>,
}
impl Bus {
    pub fn new() -> Self {
        Self {
            ram: vec![0; 1 << 24],
            regions: vec![],
            writes: vec![],
            reads: vec![],
            watched: Default::default(),
            trace: vec![],
        }
    }
    pub fn map(&mut self, address: u32, bytes: &[u8], writable: bool) {
        let end = address + bytes.len() as u32;
        assert!(end <= 1 << 24 && !bytes.is_empty());
        assert!(
            self.regions
                .iter()
                .all(|&(lo, hi, _)| address >= hi || end <= lo),
            "mapping overlap at ${address:06x}"
        );
        self.ram[address as usize..end as usize].copy_from_slice(bytes);
        self.regions.push((address, end, writable));
    }
    pub fn value(&self, address: u32, bytes: usize) -> u32 {
        self.ram[address as usize..address as usize + bytes]
            .iter()
            .enumerate()
            .fold(0, |acc, (i, b)| acc | u32::from(*b) << (i * 8))
    }
    pub fn load(&mut self, image: &Image) {
        image.verify().unwrap();
        for s in &image.segments {
            self.map(s.address, &s.bytes, s.writable);
        }
        for z in &image.zero_fill {
            self.map(z.address, &vec![0; z.size as usize], z.writable);
        }
    }
}
impl cpu::Bus for Bus {
    type Error = String;
    fn cycle(&mut self, cycle: cpu::Cycle) -> Result<u8, String> {
        if self.watched.contains(&cycle.address) && cycle.access != Access::Idle {
            self.trace.push((cycle.number, cycle.address, cycle.access));
        }
        match cycle.access {
            Access::Idle => Ok(0),
            Access::Read => {
                if !self
                    .regions
                    .iter()
                    .any(|&(lo, hi, _)| cycle.address >= lo && cycle.address < hi)
                {
                    return Err(format!(
                        "unmapped read ${:06x} cycle {}",
                        cycle.address, cycle.number
                    ));
                }
                self.reads.push(cycle.address);
                Ok(self.ram[cycle.address as usize])
            }
            Access::Write(value) => {
                if !self
                    .regions
                    .iter()
                    .any(|&(lo, hi, w)| w && cycle.address >= lo && cycle.address < hi)
                {
                    return Err(format!(
                        "unmapped/read-only write ${:06x} cycle {}",
                        cycle.address, cycle.number
                    ));
                }
                self.ram[cycle.address as usize] = value;
                self.writes.push((cycle.address, value));
                Ok(0)
            }
        }
    }
}

pub struct Harness {
    pub cpu: Machine,
    pub bus: Bus,
    pub domain_tail: Vec<u8>,
}
impl Harness {
    pub fn new(image: &Image, caller: &[u8], irq_mask: u8) -> Self {
        let mut bus = Bus::new();
        bus.load(image);
        bus.map(0x040000, caller, false);
        bus.map(0x048000, &[0xdb, 0xea], false); // platform fault sink: STP
        bus.map(0x2000, &[0; 256], true);
        bus.ram[0x2000..0x2040].fill(0xcc);
        bus.ram[0x2043] = 2; // bootstrap domain, no task owner; reserved bytes are zero
        bus.ram[0x2044..0x2046].copy_from_slice(&0x4019u16.to_le_bytes());
        bus.ram[0x2046..0x2048].copy_from_slice(&0x5ff0u16.to_le_bytes());
        bus.map(0x4000, &[0xa5; 0x2000], true);
        bus.map(0x7000, &[0; 0x400], true);
        let domain_tail = bus.ram[0x2040..0x2100].to_vec();
        let registers = Registers {
            a: 0xabcd,
            x: 0x5678,
            y: 0x9abc,
            s: 0x5ff0,
            d: 0x2000,
            dbr: 0,
            pbr: 4,
            pc: 0,
            p: irq_mask & 4,
            emulation_mode: false,
        };
        Self {
            cpu: Machine::start_at(registers),
            bus,
            domain_tail,
        }
    }
    pub fn run(&mut self) {
        assert!(
            self.cpu
                .run_until(
                    &mut self.bus,
                    2_000_000,
                    |_| Inputs::default(),
                    |cpu| cpu.is_stopped()
                )
                .unwrap(),
            "execution budget exhausted: {:?}",
            self.cpu.registers()
        );
        assert_ne!(
            self.cpu.pc() & 0xffff,
            0x8001,
            "unexpected stack-overflow fault"
        );
    }
    pub fn guards(&self, irq_mask: u8) {
        let r = self.cpu.registers();
        assert_eq!(r.s, 0x5ff0);
        assert_eq!(r.d, 0x2000);
        assert_eq!(r.dbr, 0);
        assert!(!r.emulation_mode);
        assert_eq!(r.p & 0x3c, irq_mask & 4);
        assert_eq!(&self.bus.ram[0x2040..0x2100], self.domain_tail);
        assert!(self.bus.ram[0x4000..0x401a].iter().all(|&b| b == 0xa5));
        assert!(self.bus.ram[0x5ff1..0x6000].iter().all(|&b| b == 0xa5));
    }
    pub fn global(&self, image: &Image, name: &str, bytes: usize) -> u32 {
        let data = image
            .data
            .iter()
            .find(|d| {
                d.name.eq_ignore_ascii_case(name)
                    || d.name
                        .to_ascii_uppercase()
                        .contains(&format!("_{}_", name.to_ascii_uppercase()))
            })
            .unwrap_or_else(|| panic!("missing global {name}: {:?}", image.data));
        self.bus.value(data.address, bytes)
    }
}
pub fn caller(entry: u32) -> Vec<u8> {
    assemble(
        &format!(
            "tsc\nsec\nsbc #1\ntcs\nsep #$20\n.a8\nlda #0\nsta 1,s\nrep #$20\n.a16\njsl ${entry:06x}\ntay\ntsc\nclc\nadc #1\ntcs\ntya\nstp\nnop"
        ),
        0x040000,
    )
}

pub mod context;
