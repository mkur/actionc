use super::*;
use actionc::mir65816::{
    abi,
    context::{Domain, DomainKind, FirstTask},
    image::AssemblyImport,
};
use std::collections::BTreeMap;

pub const IRQ_DP: u16 = 0x2300;
pub const IRQ_TOP: u16 = 0x6ff0;
pub const IRQ_ACK: u32 = 0x7800;
pub const NMI_ACK: u32 = 0x7801;
pub const DONE: u32 = 0x7802;
pub const FAULT: u32 = 0x048000;

pub fn routine(image: &Image, name: &str) -> u32 {
    image
        .routines
        .iter()
        .find(|r| {
            r.name.eq_ignore_ascii_case(name)
                || r.name
                    .to_ascii_uppercase()
                    .contains(&format!("_{}_", name.to_ascii_uppercase()))
        })
        .unwrap()
        .address
}
pub fn symbol(image: &Image, name: &str) -> u32 {
    image
        .data
        .iter()
        .find(|r| {
            r.name.eq_ignore_ascii_case(name)
                || r.name
                    .to_ascii_uppercase()
                    .contains(&format!("_{}_", name.to_ascii_uppercase()))
        })
        .unwrap()
        .address
}
pub fn runtime(dispatch: u32) -> Assembly {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = std::fs::read_to_string(root.join("runtime/65816/native-v1.s")).unwrap();
    assemble_artifact(
        &format!(
            "A816_IRQ_DP=${IRQ_DP:04x}\nA816_IRQ_STACK_TOP=${IRQ_TOP:04x}\nA816_IRQ_STACK_FLOOR=$600C\nA816_DISPATCH=${dispatch:06x}\nA816_STACK_OVERFLOW=${FAULT:06x}\nA816_TERMINAL=test_fault\nA816_NMI_ACK=${NMI_ACK:06x}\nA816_TASK_EXIT=test_exit\n{text}\n.export test_exit,test_fault\n.a16\n.i16\ntest_exit: lda #1\nsta f:${DONE:06x}\nstp\nnop\ntest_fault: lda #$EE\nsta f:${DONE:06x}\nstp\nnop"
        ),
        0x8000,
    )
}
pub struct ContextHarness<I = Image> {
    pub cpu: Machine,
    pub bus: Bus,
    pub image: I,
    pub symbols: BTreeMap<String, u32>,
    pub domains: Vec<Domain>,
    pub first: Vec<FirstTask>,
}
impl ContextHarness {
    pub fn new(source: &str, optimize: bool, task_entry: &str, arguments: &[u32]) -> Self {
        let prepared = prepare(source, optimize);
        // Runtime length/exports do not depend on the dispatch address. Link
        // exported assembly identities first, then assemble with the image map.
        let provisional = runtime(0x018000);
        let mut options = layout();
        for r in prepared.mir.routines.iter().filter(|r| r.entry.external) {
            let symbol = r.entry.external_symbol.unwrap();
            let (name, peak) = if symbol == actionc::nir::runtime_symbol_id("TEST.Yield") {
                ("__a816_yield_v1", 1)
            } else if symbol == actionc::nir::runtime_symbol_id("TEST.SaveIRQ") {
                ("__a816_irq_save_disable_v1", 1)
            } else if symbol == actionc::nir::runtime_symbol_id("TEST.RestoreIRQ") {
                ("__a816_irq_restore_v1", 0)
            } else {
                panic!("unexpected external {r:?}")
            };
            let address = provisional.symbols[name];
            let end = provisional
                .symbols
                .values()
                .copied()
                .filter(|&next| next > address)
                .min()
                .unwrap();
            options.imports.push(AssemblyImport {
                symbol: symbol.0,
                signature: r.signature.0,
                abi: abi::generated::ABI_NAME.into(),
                address,
                size: end - address,
                stack_peak: peak,
                checks_stack: true,
                irq_effect: match name {
                    "__a816_irq_save_disable_v1" => {
                        actionc::mir65816::image::IrqEffect::SaveDisable
                    }
                    "__a816_irq_restore_v1" => actionc::mir65816::image::IrqEffect::Restore,
                    _ => Default::default(),
                },
            });
        }
        let image = Image::from_json(&prepared.compile(&options).unwrap().image.to_json().unwrap())
            .unwrap();
        let runtime = runtime(routine(&image, "Dispatch"));
        assert_eq!(runtime.symbols, provisional.symbols);
        if let Ok(directory) = std::env::var("A816_QUALIFICATION_DIR") {
            let hash = source
                .as_bytes()
                .iter()
                .fold(0xcbf29ce484222325u64, |h, b| {
                    (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
                });
            let stem = Path::new(&directory).join(format!("context-{optimize}-{hash:016x}"));
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(stem.with_extension("act"), source).unwrap();
            std::fs::write(stem.with_extension("a816.json"), image.to_json().unwrap()).unwrap();
            std::fs::write(stem.with_extension("bridge.bin"), &runtime.bytes).unwrap();
            std::fs::write(stem.with_extension("layout.json"),serde_json::to_vec_pretty(&serde_json::json!({"bridge_origin":0x8000,"symbols":runtime.symbols,"arguments":arguments,"task_entry":task_entry,"irq_dp":IRQ_DP,"irq_stack_top":IRQ_TOP,"task_stacks":[[0x4000,0x4fff],[0x5000,0x5fff]],"task_domains":[0x2000,0x2100],"nmi_minimum_interval_cycles":250})).unwrap()).unwrap();
        }
        Self::from_loaded(image, runtime, task_entry, arguments)
    }
}

pub trait ContextImage {
    fn load_bus(&self, bus: &mut Bus);
    fn task(&self, name: &str) -> (u32, u16);
}
impl ContextImage for Image {
    fn load_bus(&self, bus: &mut Bus) {
        bus.load(self);
    }
    fn task(&self, name: &str) -> (u32, u16) {
        let address = routine(self, name);
        (
            address,
            self.routines
                .iter()
                .find(|r| r.address == address)
                .unwrap()
                .local_stack_peak as u16,
        )
    }
}
impl ContextImage for actionc::mir65816::o65::RelocatedImage {
    fn load_bus(&self, bus: &mut Bus) {
        bus.load_o65(self);
    }
    fn task(&self, name: &str) -> (u32, u16) {
        let r = self
            .profile()
            .routines
            .iter()
            .find(|r| {
                r.name.eq_ignore_ascii_case(name)
                    || r.name
                        .to_ascii_uppercase()
                        .contains(&format!("_{}_", name.to_ascii_uppercase()))
            })
            .unwrap();
        (self.routine_address(r), r.local_peak as u16)
    }
}
impl<I: ContextImage> ContextHarness<I> {
    pub fn from_loaded(image: I, runtime: Assembly, task_entry: &str, arguments: &[u32]) -> Self {
        let mut bus = Bus::new();
        image.load_bus(&mut bus);
        bus.map(0x8000, &runtime.bytes, false);
        bus.map(FAULT, &[0xdb, 0xea], false);
        bus.map(0x7800, &[0; 16], true);
        let mut vectors = [0u8; 32];
        for (offset, name) in [
            (4, "__a816_cop_v1"),
            (10, "__a816_nmi_v1"),
            (14, "__a816_irq_v1"),
        ] {
            vectors[offset..offset + 2]
                .copy_from_slice(&(runtime.symbols[name] as u16).to_le_bytes());
        }
        for offset in [6, 8] {
            vectors[offset..offset + 2]
                .copy_from_slice(&(runtime.symbols["__a816_terminal_v1"] as u16).to_le_bytes());
        }
        bus.map(0xffe0, &vectors, false);
        let irq = Domain {
            direct_page: IRQ_DP,
            owner: 0,
            kind: DomainKind::Irq,
            stack_low: 0x6000,
            stack_high: 0x6fff,
            body_s: IRQ_TOP,
            nmi_extra: 0,
        };
        bus.map(IRQ_DP.into(), &irq.bytes().unwrap(), true);
        bus.map(0x6000, &[0xa5; 0x1000], true);
        bus.map(0x7000, &[0; 0x400], true);
        let mut domains = Vec::new();
        let mut first = Vec::new();
        let (address, peak) = image.task(task_entry);
        for (i, &argument) in arguments.iter().enumerate() {
            let lo = 0x4000 + i as u16 * 0x1000;
            let domain = Domain {
                direct_page: 0x2000 + i as u16 * 0x100,
                owner: 0x7000 + i as u32 * 2,
                kind: DomainKind::Task,
                stack_low: lo,
                stack_high: lo + 0xfff,
                body_s: lo + 0xff0,
                nmi_extra: 0,
            };
            let task = FirstTask::new(
                &domain,
                address,
                argument,
                runtime.symbols["__a816_task_return_v1"],
                peak,
                false,
            )
            .unwrap();
            bus.map(domain.direct_page.into(), &domain.bytes().unwrap(), true);
            bus.map(lo.into(), &[0xa5; 0x1000], true);
            let at = usize::from(task.saved_s) + 1;
            bus.ram[at..at + 19].copy_from_slice(&task.bytes);
            bus.ram[0x7000 + i * 2..0x7002 + i * 2].copy_from_slice(&task.saved_s.to_le_bytes());
            domains.push(domain);
            first.push(task);
        }
        let restore = runtime.symbols["__a816_restore_v1"];
        let cpu = Machine::start_at(Registers {
            a: first[0].saved_s,
            s: 0x6ff0,
            d: IRQ_DP,
            p: 4,
            pc: restore as u16,
            ..Registers::default()
        });
        Self {
            cpu,
            bus,
            image,
            symbols: runtime.symbols,
            domains,
            first,
        }
    }
    pub fn tick(&mut self, inputs: Inputs) {
        self.cpu.tick(&mut self.bus, inputs).unwrap();
    }
    pub fn run(&mut self) {
        assert!(
            self.cpu
                .run_until(
                    &mut self.bus,
                    2_000_000,
                    |_| Inputs::default(),
                    |c| c.is_stopped()
                )
                .unwrap()
        );
        assert_eq!(self.bus.value(DONE, 2), 1);
    }
    pub fn guards(&self) {
        for d in &self.domains {
            let lo = usize::from(d.stack_low);
            let hi = usize::from(d.body_s);
            assert_eq!(&self.bus.ram[lo..lo + 8], &[0xa5; 8]);
            assert!(
                self.bus.ram[hi + 1..=usize::from(d.stack_high)]
                    .iter()
                    .all(|&b| b == 0xa5)
            );
            assert_eq!(
                &self.bus.ram[usize::from(d.direct_page) + 64..usize::from(d.direct_page) + 256],
                &d.bytes().unwrap()[64..]
            );
        }
        assert_eq!(&self.bus.ram[0x6000..0x6008], &[0xa5; 8]);
        assert!(self.bus.ram[0x6ff1..0x7000].iter().all(|&b| b == 0xa5));
    }
}
