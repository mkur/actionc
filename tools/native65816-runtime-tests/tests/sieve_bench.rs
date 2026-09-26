//! Original byte/bit sieve kernels and equivalent Action! ports.
//! Validate all flags against an independent full-integer prime sieve.
use actionc::mir65816::image::Image;
use actionc_vm::native65816::{self as vm, Access, Inputs, Machine, Registers};
use serde_json::{Value, json};

const DP: usize = 0x2000;
const STACK: u16 = 0x5fe0;
const RETURN: u32 = 0x040000;

fn n(v: &Value) -> usize {
    v.as_u64().unwrap().try_into().unwrap()
}
fn put(ram: &mut [u8], address: usize, size: usize, value: u32) {
    ram[address..address + size].copy_from_slice(&value.to_le_bytes()[..size]);
}
struct Bus {
    ram: Vec<u8>,
    permissions: Vec<u8>,
}
impl vm::Bus for Bus {
    type Error = String;
    fn cycle(&mut self, c: vm::Cycle) -> Result<u8, String> {
        let at = c.address as usize;
        match c.access {
            Access::Idle => Ok(0),
            Access::Read if self.permissions[at] & 1 != 0 => Ok(self.ram[at]),
            Access::Write(value) if self.permissions[at] & 2 != 0 => {
                self.ram[at] = value;
                Ok(value)
            }
            _ => Err(format!(
                "unmapped/protected access ${at:06x}: {:?}",
                c.access
            )),
        }
    }
}

fn execute(artifact: &Value, vector: &Value, mask: u8) -> Value {
    let action = artifact["compiler"] == "actionc";
    let length = n(&vector["n"]);
    let entry = n(&artifact["entry"]) as u32;
    let mut bus = Bus {
        ram: vec![0; 1 << 24],
        permissions: vec![0; 1 << 24],
    };
    if action {
        let image =
            Image::from_json(&std::fs::read(artifact["image"].as_str().unwrap()).unwrap()).unwrap();
        for segment in image.segments {
            let at = segment.address as usize;
            bus.ram[at..at + segment.bytes.len()].copy_from_slice(&segment.bytes);
        }
    } else {
        let code = std::fs::read(artifact["binary"].as_str().unwrap()).unwrap();
        bus.ram[0x10000..0x10000 + code.len()].copy_from_slice(&code);
    }
    let mut code = vec![false; 65536];
    let mut guards = vec![false; 65536];
    for r in artifact["routines"].as_array().unwrap() {
        let a = n(&r["address"]);
        let b = a + n(&r["size"]);
        assert!(0x10000 <= a && a < b && b <= 0x20000);
        code[a - 0x10000..b - 0x10000].fill(true);
        bus.permissions[a..b].fill(1);
    }
    for r in artifact["guard_ranges"].as_array().unwrap() {
        guards[n(&r[0]) - 0x10000..n(&r[1]) - 0x10000].fill(true);
    }
    bus.permissions[DP..DP + 256].fill(3);
    bus.ram[DP..DP + 256].fill(0xa5);
    if action {
        bus.ram[DP + 64..DP + 256].fill(0);
        bus.ram[DP + 0x43] = 2;
        put(&mut bus.ram, DP + 0x44, 2, 0x401a);
        put(&mut bus.ram, DP + 0x46, 2, 0x6000);
    }
    bus.permissions[0x4000..0x6000].fill(3);
    bus.ram[0x4000..0x6000].fill(0xa5);
    bus.ram[STACK as usize + 1..STACK as usize + 4].copy_from_slice(&[0xff, 0xff, 4]);
    let objects = artifact["data"].as_array().unwrap();
    let flags = objects.iter().find(|d| d["name"] == "flags").unwrap();
    let bitv = objects.iter().find(|d| d["name"] == "bitv");
    let address = n(&flags["address"]);
    let capacity = n(&flags["size"]);
    bus.ram[address..address + capacity].fill(0xa5);
    if !bitv.is_some_and(|t| n(&t["address"]) == address + capacity) {
        bus.ram[address + capacity] = 0xa5;
    }
    bus.permissions[address..address + capacity].fill(3);
    // Ordinary C byte objects may use widened non-volatile reads. The one-byte
    // halo is read-only, counted separately, and may never be modified.
    bus.permissions[address + capacity] = 1;
    if let Some(table) = bitv {
        let at = n(&table["address"]);
        assert_eq!(n(&table["size"]), 8);
        bus.permissions[at..at + 9].fill(1);
        assert_eq!(bus.ram[at..at + 8], [1, 2, 4, 8, 16, 32, 64, 128]);
        bus.ram[at + 8] = 0xa5;
    }
    let halo_before = bus.ram[address + capacity];
    let mut expected = vec![0xa5; capacity];
    for (i, v) in vector["flags"].as_array().unwrap().iter().enumerate() {
        expected[i] = n(v) as u8;
    }
    let mut args_end = STACK as usize + 4;
    if action {
        let args = artifact["arguments"].as_array().unwrap();
        assert_eq!(args.len(), 1);
        assert_eq!(n(&args[0]["size"]), 2);
        let at = STACK as usize + 4 + n(&args[0]["offset"]);
        put(&mut bus.ram, at, 2, length as u32);
        args_end = at + 2;
    }
    let original_dp = bus.ram[DP..DP + 256].to_vec();
    let mut cpu = Machine::start_at(Registers {
        a: if action { 0xabcd } else { length as u16 },
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
    // Instructions, cycles, guard cycles, stack R/W, DP R/W, REP/SEP, flags R/W, halo R.
    let mut counts = [0u64; 11];
    let mut sites = vec![[0u64; 2]; 65536];
    let mut opcodes = [[0u64; 2]; 256];
    let mut lowest = STACK;
    let mut current = 0;
    let mut opcode = 0;
    let mut guarded = false;
    let mut errors = vec![];
    for _ in 0..200_000_000 {
        if cpu.is_instruction_boundary() {
            let pc = cpu.pc();
            if pc == RETURN {
                break;
            }
            if !(0x10000..0x20000).contains(&pc) || !code[(pc & 0xffff) as usize] {
                errors.push(format!("unexpected execution ${pc:06x}"));
                break;
            }
            current = (pc & 0xffff) as usize;
            opcode = bus.ram[pc as usize] as usize;
            guarded = guards[current];
            counts[0] += 1;
            counts[7] += u64::from(matches!(opcode, 0xc2 | 0xe2));
            sites[current][0] += 1;
            opcodes[opcode][0] += 1;
        }
        match cpu.tick(&mut bus, Inputs::default()) {
            Ok(c) => {
                counts[1] += 1;
                counts[2] += u64::from(guarded);
                sites[current][1] += 1;
                opcodes[opcode][1] += 1;
                let a = c.address as usize;
                let read = u64::from(c.access == Access::Read);
                let write = u64::from(matches!(c.access, Access::Write(_)));
                if (0x4000..0x6000).contains(&a) {
                    counts[3] += read;
                    counts[4] += write;
                }
                if (DP..DP + 256).contains(&a) {
                    counts[5] += read;
                    counts[6] += write;
                }
                if (address..address + capacity).contains(&a) {
                    counts[8] += read;
                    counts[9] += write;
                }
                let flags_halo =
                    a == address + capacity && !bitv.is_some_and(|t| a == n(&t["address"]));
                if flags_halo || bitv.is_some_and(|t| a == n(&t["address"]) + 8) {
                    counts[10] += read;
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
    if r.s != STACK + 3 || r.d != DP as u16 || r.dbr != 0 || r.emulation_mode || r.p & 0x3c != mask
    {
        errors.push(format!("ABI not restored: {r:?}"));
    }
    let preserved = if action { 64 } else { 8 };
    if bus.ram[DP + preserved..DP + 256] != original_dp[preserved..] {
        errors.push("callee-preserved DP/metadata changed".into());
    }
    if bus.ram[0x4000..0x401a].iter().any(|&v| v != 0xa5)
        || bus.ram[args_end..0x6000].iter().any(|&v| v != 0xa5)
    {
        errors.push("stack canary changed".into());
    }
    let result = r.a;
    if usize::from(result) != n(&vector["expected"]) {
        errors.push(format!("wrong count: {result}"));
    }
    if let Some(at) = bus.ram[address..address + capacity]
        .iter()
        .zip(&expected)
        .position(|(a, b)| a != b)
    {
        errors.push(format!(
            "wrong flags[{at}]: {:02x}, expected {:02x}",
            bus.ram[address + at],
            expected[at]
        ));
    }
    if bus.ram[address + capacity] != halo_before {
        errors.push("flags halo modified".into());
    }
    let mut metrics = json!({
        "compiler": artifact["compiler"], "mode": artifact["mode"], "variant": artifact["variant"],
        "placement": artifact["placement"], "n": length, "flags_bytes": capacity, "rodata_bytes": artifact["rodata_bytes"],
        "result": result, "errors": errors, "code_bytes": artifact["code_bytes"], "guard_bytes": artifact["guard_bytes"],
        "peak_below_entry_s": STACK.saturating_sub(lowest),
    });
    for (key, count) in [
        "instructions",
        "cycles",
        "guard_cycles",
        "stack_reads",
        "stack_writes",
        "dp_reads",
        "dp_writes",
        "mode_switches",
        "flags_reads",
        "flags_writes",
        "halo_reads",
    ]
    .into_iter()
    .zip(counts)
    {
        metrics[key] = json!(count);
    }
    if length >= 8191 && artifact["placement"] == "normal" {
        metrics["instruction_sites"] = sites
            .iter()
            .enumerate()
            .filter(|(_, c)| c[0] != 0)
            .map(|(at, c)| (format!("{:06X}", at + 0x10000), json!(c)))
            .collect::<serde_json::Map<_, _>>()
            .into();
        metrics["opcodes"] = opcodes
            .iter()
            .enumerate()
            .filter(|(_, c)| c[0] != 0)
            .map(|(op, c)| (format!("{op:02X}"), json!(c)))
            .collect::<serde_json::Map<_, _>>()
            .into();
    }
    metrics
}

#[test]
#[ignore = "build external artifacts with tools/compare65816/sieve.py first"]
fn compare_sieve_kernels() {
    let path = std::env::var("A816_SIEVE_MANIFEST").expect("A816_SIEVE_MANIFEST");
    let manifest: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let output = std::env::var("A816_SIEVE_RESULTS").expect("A816_SIEVE_RESULTS");
    let mut records = vec![];
    let mut expected_records = 0;
    let mut failed = false;
    for artifact in manifest["artifacts"].as_array().unwrap() {
        let vectors = manifest["vectors"][artifact["variant"].as_str().unwrap()]
            .as_array()
            .unwrap();
        expected_records += vectors.len();
        for vector in vectors {
            let a = execute(artifact, vector, 0);
            let b = execute(artifact, vector, 4);
            assert_eq!(a, b, "incoming I changed behavior");
            failed |= !a["errors"].as_array().unwrap().is_empty();
            eprintln!(
                "{} {}/{} n={}: {} cycles; {}",
                artifact["variant"],
                artifact["compiler"],
                artifact["placement"],
                vector["n"],
                a["cycles"],
                a["errors"]
            );
            records.push(a);
            std::fs::write(
                &output,
                serde_json::to_vec_pretty(&json!({"records": records})).unwrap(),
            )
            .unwrap();
        }
    }
    assert_eq!(records.len(), expected_records);
    assert!(!failed, "see sieve comparison results");
}
