//! Opt-in execution of the original C CRC kernels and equivalent Action! ports.
//! Aggregate counters keep the 8 KiB speed run independent of trace storage.
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
    let width = n(&artifact["width"]);
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
    let address = n(&vector["address"]);
    let data: Vec<u8> = vector["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| n(v) as u8)
        .collect();
    bus.permissions[address..address + data.len()].fill(1);
    bus.ram[address..address + data.len()].copy_from_slice(&data);
    // Unmapped neighbors detect even speculative reads past the input object.
    let mut args_end = STACK as usize + 4;
    if action {
        let args = artifact["arguments"].as_array().unwrap();
        assert_eq!(
            args.iter().map(|a| n(&a["size"])).collect::<Vec<_>>(),
            [3, 2]
        );
        for (a, bits) in args.iter().zip([address, data.len()]) {
            let at = STACK as usize + 4 + n(&a["offset"]);
            put(&mut bus.ram, at, n(&a["size"]), bits as u32);
            args_end = args_end.max(at + n(&a["size"]));
        }
    } else {
        // Calypsi normal ABI: first-fit huge pointer in _Dp[0..3], length in A16.
        put(&mut bus.ram, DP, 4, address as u32);
    }
    let original_dp = bus.ram[DP..DP + 256].to_vec();
    let mut cpu = Machine::start_at(Registers {
        a: if action { 0xabcd } else { data.len() as u16 },
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
    // Instructions, cycles, guard cycles, stack R/W, DP R/W, REP/SEP, input R.
    let mut counts = [0u64; 9];
    let mut sites = vec![[0u64; 2]; 65536];
    let mut opcodes = [[0u64; 2]; 256];
    let mut input_reads = vec![0u32; data.len()];
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
                if (address..address + data.len()).contains(&a) {
                    counts[8] += read;
                    input_reads[a - address] += read as u32;
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
    let result = (u32::from(r.a) | if width == 32 { u32::from(r.x) << 16 } else { 0 })
        & (u32::MAX >> (32 - width));
    if result != n(&vector["expected"][width.to_string()]) as u32 {
        errors.push(format!("wrong CRC: {result:08x}"));
    }
    if action && width == 8 && r.a >> 8 != 0 {
        errors.push("BYTE result was not zero extended".into());
    }
    if input_reads.iter().any(|&n| n != 1) {
        errors.push("input bytes were not each read exactly once".into());
    }
    let mut metrics = json!({
        "compiler": artifact["compiler"], "mode": artifact["mode"], "width": width, "vector": vector["name"], "length": data.len(),
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
        "input_reads",
    ]
    .into_iter()
    .zip(counts)
    {
        metrics[key] = json!(count);
    }
    if vector["name"] == "benchmark-8192" {
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
#[ignore = "build external artifacts with tools/compare65816/crc.py first"]
fn compare_crc_kernels() {
    let path = std::env::var("A816_CRC_MANIFEST").expect("A816_CRC_MANIFEST");
    let manifest: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let short = std::env::var_os("A816_CRC_SHORT").is_some();
    let allow_external = std::env::var_os("A816_CRC_ALLOW_EXTERNAL_RESULT_ERRORS").is_some();
    let output = std::env::var("A816_CRC_RESULTS").expect("A816_CRC_RESULTS");
    let mut records = vec![];
    let mut failed = false;
    for artifact in manifest["artifacts"].as_array().unwrap() {
        for vector in manifest["vectors"].as_array().unwrap() {
            if short && vector["data"].as_array().unwrap().len() > 257 {
                continue;
            }
            let a = execute(artifact, vector, 0);
            let b = execute(artifact, vector, 4);
            assert_eq!(a, b, "incoming I changed behavior");
            let errors = a["errors"].as_array().unwrap();
            let retained_external_result_error = allow_external
                && artifact["compiler"] == "calypsi"
                && errors
                    .iter()
                    .all(|e| e.as_str().unwrap().starts_with("wrong CRC:"));
            failed |= !errors.is_empty() && !retained_external_result_error;
            eprintln!(
                "CRC{} {}/{} {}: {} cycles; {}",
                artifact["width"],
                artifact["compiler"],
                artifact["mode"],
                vector["name"],
                a["cycles"],
                a["errors"]
            );
            records.push(a);
            std::fs::write(
                &output,
                serde_json::to_vec_pretty(&json!({"short": short, "allow_external_result_errors": allow_external, "records": records})).unwrap(),
            )
            .unwrap();
        }
    }
    let vectors = manifest["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| !short || v["data"].as_array().unwrap().len() <= 257)
        .count();
    assert_eq!(
        records.len(),
        manifest["artifacts"].as_array().unwrap().len() * vectors
    );
    assert!(!failed, "see CRC comparison results");
}
