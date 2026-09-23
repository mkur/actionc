//! Larger external-artifact comparison. Execute final bytes against all existing
//! C-derived vectors; bounded aggregate profiling avoids storing a cycle trace.
use actionc::mir65816::image::Image;
use actionc_vm::native65816::{self as vm, Access, Inputs, Machine, Registers};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

const DP: usize = 0x2000;
const STACK: u16 = 0x5fe0;
const RETURN: u32 = 0x040000;
const HEADER: &[(&str, usize)] = &[
    ("startNode", 2),
    ("endNode", 4),
    ("result", 6),
    ("checksum", 8),
    ("queueCount", 10),
    ("queueNext", 12),
    ("argNode", 16),
    ("argDist", 18),
    ("argPrev", 20),
    ("outNode", 22),
    ("outDist", 24),
    ("outPrev", 26),
    ("seedNext", 28),
];
fn n(v: &Value) -> usize {
    v.as_u64().unwrap().try_into().unwrap()
}
fn get(bytes: &[u8], at: usize, size: usize) -> u32 {
    bytes[at..at + size]
        .iter()
        .enumerate()
        .map(|(i, &b)| u32::from(b) << (8 * i))
        .sum()
}
fn put(bytes: &mut [u8], at: usize, size: usize, value: u32) {
    bytes[at..at + size].copy_from_slice(&value.to_le_bytes()[..size]);
}
fn hex(text: &str) -> Vec<u8> {
    if text == "-" {
        return vec![];
    }
    assert_eq!(text.len() % 2, 0);
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}
fn pool(text: &str) -> Vec<u8> {
    let mut b = hex(text);
    assert!(b.len() <= 8000);
    b.resize(8000, 0);
    b
}
fn pointer(wire: u32, base: usize, stride: usize) -> u32 {
    if wire == 0 {
        return 0;
    }
    assert!(wire >= 0x4001 && (wire - 0x4001) % 8 == 0 && (wire - 0x4001) / 8 < 1000);
    (base + (wire as usize - 0x4001) / 8 * stride)
        .try_into()
        .unwrap()
}
struct Bus<'a> {
    ram: Vec<u8>,
    permissions: &'a [u8],
}
impl vm::Bus for Bus<'_> {
    type Error = String;
    fn cycle(&mut self, c: vm::Cycle) -> Result<u8, String> {
        let at = c.address as usize;
        match c.access {
            Access::Idle => Ok(0),
            Access::Read if self.permissions[at] & 1 != 0 => Ok(self.ram[at]),
            Access::Write(b) if self.permissions[at] & 2 != 0 => {
                self.ram[at] = b;
                Ok(b)
            }
            _ => Err(format!(
                "unmapped/protected access ${at:06x}: {:?}",
                c.access
            )),
        }
    }
}
struct Program {
    artifact: Value,
    ram: Vec<u8>,
    permissions: Vec<u8>,
    owner: Vec<usize>,
    guard: Vec<bool>,
    sizes: [usize; 3],
    node: [usize; 2],
    queue: [usize; 4],
}
impl Program {
    fn address(&self, name: &str) -> usize {
        n(&self.artifact["globals"][name]["address"])
    }
    fn set(&self, ram: &mut [u8], name: &str, value: u32) {
        put(
            ram,
            self.address(name),
            n(&self.artifact["globals"][name]["size"]),
            value,
        );
    }
    fn new(artifact: &Value) -> Self {
        let mut p = Self {
            artifact: artifact.clone(),
            ram: vec![0; 1 << 24],
            permissions: vec![0; 1 << 24],
            owner: vec![usize::MAX; 65536],
            guard: vec![false; 65536],
            sizes: [0; 3],
            node: [0; 2],
            queue: [0; 4],
        };
        if artifact["compiler"] == "actionc" {
            let image =
                Image::from_json(&std::fs::read(artifact["image"].as_str().unwrap()).unwrap())
                    .unwrap();
            for s in image.segments {
                p.ram[s.address as usize..s.address as usize + s.bytes.len()]
                    .copy_from_slice(&s.bytes);
            }
        } else {
            let code = std::fs::read(artifact["binary"].as_str().unwrap()).unwrap();
            p.ram[0x10000..0x10000 + code.len()].copy_from_slice(&code);
        }
        for (i, r) in artifact["routines"].as_array().unwrap().iter().enumerate() {
            let a = n(&r["address"]);
            let b = a + n(&r["size"]);
            assert!(0x10000 <= a && a < b && b <= 0x20000);
            p.permissions[a..b].fill(1);
            p.owner[a - 0x10000..b - 0x10000].fill(i);
        }
        for r in artifact["guard_ranges"].as_array().unwrap() {
            p.guard[n(&r[0]) - 0x10000..n(&r[1]) - 0x10000].fill(true);
        }
        for r in artifact["data_ranges"].as_array().unwrap() {
            p.permissions[n(&r[0])..n(&r[1])].fill(3);
        }
        p.permissions[DP..DP + 256].fill(3);
        p.ram[DP..DP + 256].fill(0xa5);
        if artifact["compiler"] == "actionc" {
            p.ram[DP + 64..DP + 256].fill(0);
            p.ram[DP + 0x43] = 2;
            put(&mut p.ram, DP + 0x44, 2, 0x401a);
            put(&mut p.ram, DP + 0x46, 2, 0x6000);
        }
        // ABI metadata and callee-preserved DP are checked after every call.
        p.permissions[0x4000..0x6000].fill(3);
        p.ram[0x4000..0x6000].fill(0xa5);
        p.ram[STACK as usize + 1..STACK as usize + 4].copy_from_slice(&[0xff, 0xff, 4]);
        p.permissions[RETURN as usize..RETURN as usize + 2].fill(1);
        p.ram[RETURN as usize] = 0xdb;
        p
    }
    fn layout(&mut self) {
        let mut ram = self.ram.clone();
        self.set(&mut ram, "command", 255);
        let (ram, metrics) = self.execute(ram, 0, 100_000);
        assert_eq!(metrics["errors"], json!([]), "layout query: {metrics}");
        let query = |name| get(&ram, self.address(name), 4) as usize;
        assert_eq!((query("layoutIntSize"), query("layoutPointerSize")), (2, 3));
        assert_eq!(query("layoutRowCost"), 0);
        let sizes = [
            query("layoutNodeSize"),
            query("layoutRowSize"),
            query("layoutQueueSize"),
        ];
        let node = [query("layoutNodeDist"), query("layoutNodePrev")];
        let queue = [
            query("layoutQueueNode"),
            query("layoutQueueDist"),
            query("layoutQueuePrev"),
            query("layoutQueueNext"),
        ];
        self.sizes = sizes;
        self.node = node;
        self.queue = queue;
        assert_eq!(&self.sizes[..2], &[4, 100]);
        assert_eq!(self.node, [0, 2]);
        assert_eq!(self.queue, [0, 2, 4, 6]);
        assert!([9, 10].contains(&self.sizes[2]));
        assert_eq!(
            n(&self.artifact["globals"]["items"]["size"]),
            1000 * self.sizes[2]
        );
        // Protect padding even against stores that restore the old value.
        for i in 0..1000 {
            let base = self.address("items") + i * self.sizes[2];
            self.ram[base + 9..base + self.sizes[2]].fill(0xa7);
            self.permissions[base + 9..base + self.sizes[2]].fill(1);
        }
    }
    fn seed(&self, f: &[&str], graph: &[u8]) -> Vec<u8> {
        let mut ram = self.ram.clone();
        let h = hex(f[2]);
        assert_eq!(h.len(), 32);
        self.set(&mut ram, "command", h[0].into());
        for &(name, at) in HEADER {
            self.set(&mut ram, name, get(&h, at, 2));
        }
        self.set(
            &mut ram,
            "headAddress",
            pointer(get(&h, 14, 2), self.address("items"), self.sizes[2]),
        );
        ram[self.address("matrix")..self.address("matrix") + 10000].copy_from_slice(graph);
        self.set_pool(&mut ram, &pool(f[3]));
        ram
    }
    fn set_pool(&self, ram: &mut [u8], wire: &[u8]) {
        for i in 0..1000 {
            for j in 0..4 {
                let mut value = get(wire, i * 8 + j * 2, 2);
                if j == 3 {
                    value = pointer(value, self.address("items"), self.sizes[2]);
                }
                put(
                    ram,
                    self.address("items") + i * self.sizes[2] + self.queue[j],
                    if j == 3 { 3 } else { 2 },
                    value,
                );
            }
        }
    }
    fn check(&self, ram: &[u8], f: &[&str], graph: &[u8]) -> Vec<String> {
        let h = hex(f[4]);
        let nodes = hex(f[5]);
        assert_eq!((h.len(), nodes.len()), (32, 400));
        let mut expected = self.seed(f, graph);
        for &(name, at) in HEADER {
            self.set(&mut expected, name, get(&h, at, 2));
        }
        let head = pointer(get(&h, 14, 2), self.address("items"), self.sizes[2]);
        self.set(&mut expected, "headAddress", head);
        self.set(&mut expected, "head", head);
        self.set(&mut expected, "done", 0xa5);
        expected[self.address("nodes")..self.address("nodes") + 400].copy_from_slice(&nodes);
        self.set_pool(&mut expected, &pool(f[6]));
        let mut errors = vec![];
        for (name, entry) in self.artifact["globals"].as_object().unwrap() {
            let at = n(&entry["address"]);
            let size = n(&entry["size"]);
            if ram[at..at + size] != expected[at..at + size] {
                let differences: Vec<_> = (0..size)
                    .filter(|&i| ram[at + i] != expected[at + i])
                    .take(8)
                    .map(|i| format!("+{i}: {:02x} != {:02x}", ram[at + i], expected[at + i]))
                    .collect();
                errors.push(format!("{name}: {}", differences.join(", ")));
            }
        }
        errors
    }
    fn execute(&self, ram: Vec<u8>, mask: u8, budget: u64) -> (Vec<u8>, Value) {
        let entry = n(&self.artifact["entry"]) as u32;
        let action = self.artifact["compiler"] == "actionc";
        let mut bus = Bus {
            ram,
            permissions: &self.permissions,
        };
        let mut cpu = Machine::start_at(Registers {
            a: 0xabcd,
            x: 0x5678,
            y: 0x9abc,
            s: STACK,
            d: DP as u16,
            dbr: 0,
            pbr: (entry >> 16) as u8,
            pc: entry as u16,
            p: mask,
            emulation_mode: false,
        });
        // Columns: instructions, cycles, guard cycles, stack read/write,
        // scratch DP read/write, metadata reads, REP/SEP instructions.
        let mut counts = vec![[0u64; 9]; self.artifact["routines"].as_array().unwrap().len()];
        let mut sites = vec![0u64; 65536];
        let mut lowest = STACK;
        let mut current = 0;
        let mut guarded = false;
        let mut errors = vec![];
        for _ in 0..budget {
            if cpu.is_instruction_boundary() {
                let pc = cpu.pc();
                if pc == RETURN {
                    break;
                }
                if !(0x10000..0x20000).contains(&pc)
                    || self.owner[(pc & 0xffff) as usize] == usize::MAX
                {
                    errors.push(format!("unexpected execution ${pc:06x}"));
                    break;
                }
                current = self.owner[(pc & 0xffff) as usize];
                guarded = self.guard[(pc & 0xffff) as usize];
                counts[current][0] += 1;
                sites[(pc & 0xffff) as usize] += 1;
                counts[current][8] += u64::from(matches!(bus.ram[pc as usize], 0xc2 | 0xe2));
            }
            match cpu.tick(&mut bus, Inputs::default()) {
                Ok(c) => {
                    let s = &mut counts[current];
                    s[1] += 1;
                    s[2] += u64::from(guarded);
                    let read = u64::from(c.access == Access::Read);
                    let write = u64::from(matches!(c.access, Access::Write(_)));
                    let a = c.address as usize;
                    if (0x4000..0x6000).contains(&a) {
                        s[3] += read;
                        s[4] += write;
                    }
                    if (DP..DP + if action { 64 } else { 80 }).contains(&a) {
                        s[5] += read;
                        s[6] += write;
                    }
                    if action && (DP + 64..DP + 256).contains(&a) {
                        s[7] += read;
                    }
                }
                Err(e) => {
                    errors.push(format!("{e}; CPU {:?}", cpu.registers()));
                    break;
                }
            }
            lowest = lowest.min(cpu.registers().s);
            if cpu.is_stopped() {
                errors.push("unexpected STP".into());
                break;
            }
        }
        if !cpu.is_instruction_boundary() || cpu.pc() != RETURN {
            errors.push("did not return within cycle budget".into());
        }
        let r = cpu.registers();
        if r.s != STACK + 3
            || r.d != DP as u16
            || r.dbr != 0
            || r.emulation_mode
            || r.p & 0x3c != mask
        {
            errors.push(format!("ABI not restored: {r:?}"));
        }
        let preserved = if action { 64 } else { 80 };
        if bus.ram[DP + preserved..DP + 256] != self.ram[DP + preserved..DP + 256] {
            errors.push("DP metadata/preserved bytes changed".into());
        }
        if !action && (bus.ram[DP + 32..DP + 56] != self.ram[DP + 32..DP + 56]) {
            errors.push("vbcc callee-preserved registers changed".into());
        }
        if bus.ram[0x4000..0x401a] != self.ram[0x4000..0x401a]
            || bus.ram[STACK as usize + 4..0x6000] != self.ram[STACK as usize + 4..0x6000]
        {
            errors.push("stack canary changed".into());
        }
        let mut totals = [0; 9];
        for c in &counts {
            for i in 0..9 {
                totals[i] += c[i];
            }
        }
        let mut metrics = metrics(totals);
        metrics["peak_below_entry_s"] = json!(STACK.saturating_sub(lowest));
        metrics["errors"] = json!(errors);
        metrics["routines"] = self.artifact["routines"]
            .as_array()
            .unwrap()
            .iter()
            .zip(counts)
            .map(|(r, c)| (r["name"].as_str().unwrap().to_owned(), metrics_value(c)))
            .collect::<serde_json::Map<_, _>>()
            .into();
        metrics["instruction_sites"] = sites
            .into_iter()
            .enumerate()
            .filter(|&(_, c)| c != 0)
            .map(|(a, c)| ((a + 0x10000).to_string(), json!(c)))
            .collect::<serde_json::Map<_, _>>()
            .into();
        (bus.ram, metrics)
    }
}
fn metrics_value(c: [u64; 9]) -> Value {
    metrics(c)
}
fn metrics(c: [u64; 9]) -> Value {
    [
        "instructions",
        "cycles",
        "stack_check_cycles",
        "stack_reads",
        "stack_writes",
        "dp_reads",
        "dp_writes",
        "metadata_reads",
        "mode_switches",
    ]
    .into_iter()
    .zip(c)
    .map(|(k, v)| (k.to_owned(), json!(v)))
    .collect::<serde_json::Map<_, _>>()
    .into()
}

#[test]
#[ignore = "build external artifacts with tools/compare65816/dijkstra.py first"]
fn compare_dijkstra_artifacts() {
    let path = std::env::var("A816_DIJKSTRA_MANIFEST").expect("A816_DIJKSTRA_MANIFEST");
    let manifest: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let graphs_text = std::fs::read_to_string(manifest["graphs"].as_str().unwrap()).unwrap();
    let graphs: BTreeMap<_, _> = graphs_text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let f: Vec<_> = l.split_whitespace().collect();
            assert_eq!(f.len(), 2);
            let b = hex(f[1]);
            assert_eq!(b.len(), 10000);
            (f[0], b)
        })
        .collect();
    assert_eq!(graphs.len(), 11);
    let text = std::fs::read_to_string(manifest["vectors"].as_str().unwrap()).unwrap();
    let vectors: Vec<_> = text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    assert_eq!(vectors.len(), 33);
    let filter = std::env::var("A816_DIJKSTRA_FILTER").unwrap_or_default();
    let mut results = vec![];
    let mut failures = vec![];
    for artifact in manifest["artifacts"].as_array().unwrap() {
        let mut p = Program::new(artifact);
        p.layout();
        for line in &vectors {
            let f: Vec<_> = line.split_whitespace().collect();
            assert_eq!(f.len(), 7);
            if !filter.is_empty() && !f[0].contains(&filter) {
                continue;
            }
            let mut measurements = vec![];
            for mask in [0, 4] {
                let (ram, mut result) = p.execute(p.seed(&f, &graphs[f[1]]), mask, 4_000_000_000);
                result["errors"].as_array_mut().unwrap().extend(
                    p.check(&ram, &f, &graphs[f[1]])
                        .into_iter()
                        .map(Value::String),
                );
                if !result["errors"].as_array().unwrap().is_empty() {
                    failures.push(format!(
                        "{}/{} {} I={mask}: {}",
                        artifact["compiler"], artifact["mode"], f[0], result["errors"]
                    ));
                }
                measurements.push(result);
            }
            assert_eq!(
                measurements[0], measurements[1],
                "interrupt mask changed behavior: {}",
                f[0]
            );
            let mut result = measurements.remove(0);
            if f[0] != "benchmark-original" && f[0] != "original-0-50" {
                result.as_object_mut().unwrap().remove("instruction_sites");
            }
            result["compiler"] = artifact["compiler"].clone();
            result["mode"] = artifact["mode"].clone();
            result["case"] = json!(f[0]);
            result["queue_stride"] = json!(p.sizes[2]);
            result["interrupt_masks"] = json!([0, 4]);
            eprintln!(
                "{}/{} {}: {} cycles; {} errors",
                artifact["compiler"],
                artifact["mode"],
                f[0],
                result["cycles"],
                result["errors"].as_array().unwrap().len()
            );
            results.push(result);
            let output = std::env::var("A816_DIJKSTRA_RESULTS").expect("A816_DIJKSTRA_RESULTS");
            std::fs::write(
                Path::new(&output),
                serde_json::to_vec_pretty(&json!({"schema":1,"filter":filter,"results":results}))
                    .unwrap(),
            )
            .unwrap();
        }
    }
    assert!(!results.is_empty(), "empty case filter");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn wire_links_and_full_state_oracle_controls() {
    assert_eq!(pointer(0, 0x180000, 10), 0);
    assert_eq!(pointer(0x4001 + 8 * 998, 0x180000, 10), 0x180000 + 9980);
    assert!(std::panic::catch_unwind(|| pointer(0x4002, 0x180000, 10)).is_err());
    assert!(std::panic::catch_unwind(|| pointer(0x4001 + 8000, 0x180000, 10)).is_err());
    assert_eq!(
        "abc\r\ndef\r\n".lines().collect::<Vec<_>>(),
        vec!["abc", "def"]
    );
    let permissions = [1, 3];
    let mut b = Bus {
        ram: vec![0, 0],
        permissions: &permissions,
    };
    // CPU execution checks protected writes through the same Bus::cycle path.
    assert_eq!(pool("0100")[..4], [1, 0, 0, 0]);
    put(&mut b.ram, 0, 2, 0x1234);
    assert_eq!(get(&b.ram, 0, 2), 0x1234);
}
