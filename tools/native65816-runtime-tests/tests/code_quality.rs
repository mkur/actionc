//! Opt-in comparison of external compiler artifacts. This is not a golden
//! assembly test: correctness is checked against independently generated inputs.
mod support;

use actionc::mir65816::image::Image;
use actionc_vm::native65816::{Access, Inputs, Machine, Registers};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
use support::{Bus, homes, word_edge};

const ENTRY_S: u16 = 0x5fe0;
const RETURN: u32 = 0x040000;
const FAULT: u32 = 0x048000;
const DP: usize = 0x2000;

fn number(value: &Value) -> u32 {
    value.as_u64().unwrap().try_into().unwrap()
}

fn bytes(value: &Value) -> Vec<u8> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| number(v).try_into().unwrap())
        .collect()
}

fn contains(ranges: &Value, address: u32) -> bool {
    ranges
        .as_array()
        .unwrap()
        .iter()
        .any(|r| number(&r[0]) <= address && address < number(&r[1]))
}

fn execute(
    artifact: &Value,
    case: &Value,
    input: &Value,
    mask: u8,
    sites: &support::word_edge::Index,
    forwarded: &support::forwarding::Index,
) -> Value {
    let action = artifact["compiler"] == "actionc";
    let mut bus = Bus::new();
    bus.single_word_edges = sites.clone();
    bus.forwarded_words = forwarded.clone();
    let mut native_routines = vec![];
    if action {
        let serialized = std::fs::read(artifact["image"].as_str().unwrap()).unwrap();
        let image = Image::from_json(&serialized).unwrap();
        native_routines = image.routines.clone();
        bus.load(&image);
    } else {
        let code = std::fs::read(artifact["binary"].as_str().unwrap()).unwrap();
        bus.map(0x010000, &code, false);
    }
    bus.map(RETURN, &[0xdb, 0xea], false);
    bus.map(FAULT, &[0xdb, 0xea], false);
    bus.map(
        DP as u32,
        &(0..256)
            .map(|i| (i as u8).wrapping_mul(37).wrapping_add(0x93))
            .collect::<Vec<_>>(),
        true,
    );
    if action {
        bus.ram[DP + 64..DP + 256].fill(0);
        bus.ram[DP + 0x43] = 2;
        bus.ram[DP + 0x44..DP + 0x46].copy_from_slice(&0x401au16.to_le_bytes());
        bus.ram[DP + 0x46..DP + 0x48].copy_from_slice(&0x6000u16.to_le_bytes());
    }
    let original_dp = bus.ram[DP..DP + 256].to_vec();
    bus.map(0x4000, &[0xa5; 0x2000], true);
    let argument_base = usize::from(ENTRY_S) + 4;
    // RTL increments the 16-bit return PC within its bank.
    bus.ram[usize::from(ENTRY_S) + 1..argument_base].copy_from_slice(&[0xff, 0xff, 4]);
    let entry = number(&artifact["entry"]);
    let mut registers = Registers {
        a: 0xabcd,
        x: 0x5678,
        y: 0x9abc,
        s: ENTRY_S,
        d: DP as u16,
        dbr: 0,
        pbr: (entry >> 16) as u8,
        pc: entry as u16,
        p: mask,
        emulation_mode: false,
    };
    let arguments = artifact["arguments"].as_array().unwrap();
    let values = input["args"].as_array().unwrap();
    assert_eq!(arguments.len(), values.len());
    let mut stack_arguments = 0;
    for (i, (argument, value)) in arguments.iter().zip(values).enumerate() {
        let value = number(value);
        let size = number(&argument["size"]) as usize;
        if !action && i == 0 {
            registers.a = value as u16;
            if size > 2 {
                registers.x = (value >> 16) as u16;
            }
        } else {
            let offset = if action {
                number(&argument["offset"]) as usize
            } else {
                stack_arguments
            };
            bus.ram[argument_base + offset..argument_base + offset + size]
                .copy_from_slice(&value.to_le_bytes()[..size]);
            stack_arguments = offset
                + if action {
                    size
                } else {
                    number(&argument["slot_size"]) as usize
                };
        }
    }
    if action {
        // Incoming stack extent includes the ABI's final odd-size padding.
        stack_arguments |= 1;
    }
    let preserved_stack_start = argument_base + stack_arguments;
    let mut logical_memory = BTreeSet::new();
    let mut padding = BTreeSet::new();
    for region in input["memory"].as_array().unwrap() {
        let address = number(&region["address"]);
        let data = bytes(&region["bytes"]);
        let mut guarded = vec![0xa7];
        guarded.extend_from_slice(&data);
        guarded.extend_from_slice(&[0x5d, 0xe3]);
        bus.map(address - 1, &guarded, true);
        logical_memory.extend(address..address + data.len() as u32);
        padding.extend([
            address - 1,
            address + data.len() as u32,
            address + data.len() as u32 + 1,
        ]);
    }
    let mut cpu = Machine::start_at(registers);
    let mut lowest_s = ENTRY_S;
    let mut instructions = 0u64;
    let mut instruction_sites = BTreeMap::<u32, u64>::new();
    let mut fused_sites = BTreeMap::<u32, u64>::new();
    let mut word_edge_sites = BTreeMap::<u32, u64>::new();
    let mut edge_words = 0u64;
    let mut direct_sites = BTreeMap::<u32, u64>::new();
    let mut acyclic_sites = BTreeMap::<u32, u64>::new();
    let mut acyclic_words = 0u64;
    let mut coalesced_sites = BTreeMap::<u32, u64>::new();
    let mut selective_sites = BTreeMap::<u32, u64>::new();
    let mut selective_words = 0u64;
    let mut selective_staged = 0u64;
    let mut parameter_forwarded_sites = BTreeMap::<u32, u64>::new();
    let mut frame_forwarded_sites = BTreeMap::<u32, u64>::new();
    let mut x_sites = BTreeMap::<u32, u64>::new();
    let mut increment_sites = BTreeMap::<u32, u64>::new();
    let mut forwarded_sites = BTreeMap::<u32, u64>::new();
    let mut guard_cycles = 0u64;
    let mut guard_instructions = 0u64;
    let mut in_guard = false;
    let mut instruction_pc = entry;
    let mut last_memory_write = BTreeMap::new();
    let mut stack_reads = 0u64;
    let mut stack_writes = 0u64;
    let mut dp_reads = 0u64;
    let mut dp_writes = 0u64;
    let mut metadata_reads = 0u64;
    let mut padding_reads = 0u64;
    let mut dp_touched = BTreeSet::new();
    let scratch_end = DP as u32 + if action { 64 } else { 80 };
    for _ in 0..2_000_000 {
        if cpu.is_instruction_boundary() {
            let pc = cpu.pc();
            if pc == RETURN {
                break;
            }
            assert_ne!(pc, FAULT, "unexpected stack guard fault");
            assert!(
                contains(&artifact["code_ranges"], pc),
                "execution outside counted code at ${pc:06x}"
            );
            if let Some(window) = support::comparison::fused_window(&cpu, &bus, &native_routines) {
                *fused_sites.entry(window.load).or_default() += 1;
            }
            if let Some(window) = support::word_edge::reached(&cpu, &bus, &native_routines) {
                *word_edge_sites.entry(cpu.pc()).or_default() += 1;
                edge_words += window.moves.len() as u64;
                if window.form == word_edge::Form::Direct {
                    let count = window.moves.len() - window.order.len();
                    if count > 0 {
                        *coalesced_sites.entry(pc).or_default() += count as u64;
                    }
                }
                if window.form == word_edge::Form::Direct && window.moves.len() == 1 {
                    *direct_sites.entry(cpu.pc()).or_default() += 1;
                }
                if window.form == word_edge::Form::Direct && window.moves.len() > 1 {
                    *acyclic_sites.entry(cpu.pc()).or_default() += 1;
                    acyclic_words += window.moves.len() as u64;
                }
                if window.form == word_edge::Form::Selective {
                    *selective_sites.entry(cpu.pc()).or_default() += 1;
                    selective_words += window.moves.len() as u64;
                    selective_staged +=
                        window.moves.iter().filter(|m| m.1.is_some()).count() as u64;
                }
            }
            if let Some(site) = support::forwarding::reached(&cpu, &bus) {
                let r = cpu.registers();
                assert_eq!(
                    u32::from(r.a),
                    bus.value(homes::address(r.s, r.d, site.slot), 2)
                );
                assert_eq!(
                    r.p & 0x82,
                    if r.a == 0 { 2 } else { 0 } | if r.a & 0x8000 != 0 { 0x80 } else { 0 }
                );
                *forwarded_sites.entry(pc).or_default() += 1;
            }
            if let Some(site) = support::frame_forwarding::reached(&cpu, &bus) {
                site.assert_live(&cpu, &bus);
                *frame_forwarded_sites.entry(pc).or_default() += 1;
            }
            if let Some(site) = support::parameter_forwarding::reached(&cpu, &bus) {
                site.assert_live(&cpu, &bus);
                *parameter_forwarded_sites.entry(pc).or_default() += 1;
            }
            if let Some(site) = support::x_residency::reached(&cpu, &bus) {
                site.assert_live(&cpu, &bus);
                *x_sites.entry(pc).or_default() += 1;
            }
            if let Some(site) = support::x_residency::increment(&cpu, &bus) {
                site.assert_live(&cpu, &bus);
                *increment_sites.entry(pc).or_default() += 1;
            }
            instructions += 1;
            *instruction_sites.entry(pc).or_default() += 1;
            instruction_pc = pc;
            in_guard = contains(&artifact["guard_ranges"], pc);
            guard_instructions += u64::from(in_guard);
        }
        let cycle = cpu
            .tick(&mut bus, Inputs::default())
            .unwrap_or_else(|error| panic!("{error}; CPU {:?}", cpu.registers()));
        guard_cycles += u64::from(in_guard);
        lowest_s = lowest_s.min(cpu.registers().s);
        let read = cycle.access == Access::Read;
        let write = matches!(cycle.access, Access::Write(_));
        if write && logical_memory.contains(&cycle.address) {
            last_memory_write.insert(cycle.address, instruction_pc);
        }
        if (0x4000..0x6000).contains(&cycle.address) {
            stack_reads += u64::from(read);
            stack_writes += u64::from(write);
        }
        if (DP as u32..scratch_end).contains(&cycle.address) {
            dp_reads += u64::from(read);
            dp_writes += u64::from(write);
            if read || write {
                dp_touched.insert(cycle.address - DP as u32);
            }
        }
        if action && (scratch_end..DP as u32 + 256).contains(&cycle.address) {
            metadata_reads += u64::from(read);
        }
        padding_reads += u64::from(read && padding.contains(&cycle.address));
        assert!(
            !(write && padding.contains(&cycle.address)),
            "write past input object"
        );
        assert!(!cpu.is_stopped(), "unexpected STP");
    }
    assert!(
        cpu.is_instruction_boundary() && cpu.pc() == RETURN,
        "cycle budget exhausted"
    );
    let returned = cpu.registers();
    assert_eq!(returned.s, ENTRY_S + 3);
    assert_eq!(returned.d, DP as u16);
    assert_eq!(returned.dbr, 0);
    assert!(!returned.emulation_mode);
    assert_eq!(returned.p & 0x3c, mask);
    if action {
        assert_eq!(&bus.ram[DP + 64..DP + 256], &original_dp[64..]);
    } else {
        assert_eq!(
            &bus.ram[DP + 32..DP + 56],
            &original_dp[32..56],
            "callee-preserved r16..r27"
        );
        assert_eq!(&bus.ram[DP + 80..DP + 256], &original_dp[80..]);
    }
    assert!(bus.ram[0x4000..0x401a].iter().all(|&b| b == 0xa5));
    assert!(
        bus.ram[preserved_stack_start..0x6000]
            .iter()
            .all(|&b| b == 0xa5)
    );
    let result = match number(&case["returns"]) {
        0 => Value::Null,
        2 => json!(returned.a),
        4 => json!(u32::from(returned.a) | u32::from(returned.x) << 16),
        other => panic!("unsupported result width {other}"),
    };
    let mut errors = Vec::new();
    if result != input["result"] {
        errors.push(format!(
            "scalar result: expected {}, got {result}",
            input["result"]
        ));
    }
    // Verify every input object, including inputs that should remain unchanged.
    for before in input["memory"].as_array().unwrap() {
        let expected = input["after"]
            .as_array()
            .unwrap()
            .iter()
            .find(|after| after["address"] == before["address"])
            .unwrap_or(before);
        let address = number(&expected["address"]) as usize;
        let expected = bytes(&expected["bytes"]);
        for (i, expected) in expected.into_iter().enumerate() {
            let actual = bus.ram[address + i];
            if actual != expected {
                let at = (address + i) as u32;
                errors.push(format!("memory ${at:06x}: expected ${expected:02x}, got ${actual:02x}; last store PC ${:06x}", last_memory_write[&at]));
            }
        }
    }
    let mut measurement = json!({
        "cycles": cpu.cycles(), "instructions": instructions, "instruction_sites": instruction_sites,
        "stack_check_cycles": guard_cycles, "stack_check_instructions": guard_instructions,
        "peak_below_entry_s": ENTRY_S-lowest_s,
        "incoming_stack_bytes": stack_arguments, "incoming_return_bytes": 3,
        "stack_reads": stack_reads, "stack_writes": stack_writes,
        "dp_reads": dp_reads, "dp_writes": dp_writes, "dp_touched_offsets": dp_touched,
        "metadata_reads": metadata_reads, "input_padding_reads": padding_reads,
        "result": result, "correct": errors.is_empty(), "errors": errors
    });
    if action {
        measurement["x_increment_updates"] = json!(increment_sites.values().sum::<u64>());
        measurement["x_increment_update_sites"] = json!(increment_sites);
        measurement["x_forwarded_loads"] = json!(x_sites.values().sum::<u64>());
        measurement["x_forwarded_load_sites"] = json!(x_sites);
        measurement["fused_branches"] = json!(fused_sites.values().sum::<u64>());
        measurement["fused_branch_sites"] = json!(fused_sites);
        measurement["word_edges"] = json!(word_edge_sites.values().sum::<u64>());
        measurement["edge_words"] = json!(edge_words);
        measurement["word_edge_sites"] = json!(word_edge_sites);
        measurement["direct_word_edges"] = json!(direct_sites.values().sum::<u64>());
        measurement["direct_word_edge_sites"] = json!(direct_sites);
        measurement["acyclic_word_edges"] = json!(acyclic_sites.values().sum::<u64>());
        measurement["acyclic_edge_words"] = json!(acyclic_words);
        measurement["coalesced_word_copies"] = json!(coalesced_sites.values().sum::<u64>());
        measurement["coalesced_word_copy_sites"] = json!(coalesced_sites);
        measurement["acyclic_word_edge_sites"] = json!(acyclic_sites);
        measurement["selective_word_edges"] = json!(selective_sites.values().sum::<u64>());
        measurement["selective_edge_words"] = json!(selective_words);
        measurement["selective_staged_words"] = json!(selective_staged);
        measurement["selective_direct_words"] = json!(selective_words - selective_staged);
        measurement["selective_word_edge_sites"] = json!(selective_sites);
        measurement["parameter_forwarded_loads"] =
            json!(parameter_forwarded_sites.values().sum::<u64>());
        measurement["parameter_forwarded_load_sites"] = json!(parameter_forwarded_sites);
        measurement["frame_forwarded_loads"] = json!(frame_forwarded_sites.values().sum::<u64>());
        measurement["frame_forwarded_load_sites"] = json!(frame_forwarded_sites);
        measurement["forwarded_word_loads"] = json!(forwarded_sites.values().sum::<u64>());
        measurement["forwarded_word_load_sites"] = json!(forwarded_sites);
    }
    measurement
}

#[test]
#[ignore = "requires build.py artifacts and A816_COMPARISON_MANIFEST"]
fn execute_parallel_corpus() {
    let path = PathBuf::from(
        std::env::var_os("A816_COMPARISON_MANIFEST").expect("set A816_COMPARISON_MANIFEST"),
    );
    let destination = std::env::var_os("A816_COMPARISON_RESULTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| path.parent().unwrap().join("measurements.json"));
    if destination.exists() {
        // An aborted run must not leave previous measurements looking current.
        std::fs::remove_file(&destination).unwrap();
    }
    let manifest: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(manifest["schema"], 1);
    assert_eq!(manifest["target"], "wdc-65816-native");
    let mut measurements = Vec::new();
    let mut control = Vec::new();
    for artifact in manifest["artifacts"].as_array().unwrap() {
        let case = manifest["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["id"] == artifact["case"])
            .unwrap();
        // Ground short copy decoding in typed edges from an independently
        // re-prepared artifact. CPU execution still uses only the saved image.
        let (sites, forwarded) = if artifact["compiler"] == "actionc" {
            let command = artifact["commands"][0].as_array().unwrap();
            let source = command.last().unwrap().as_str().unwrap();
            let layout_pos = command.iter().position(|v| v == "--layout").unwrap();
            let options = serde_json::from_slice(
                &std::fs::read(command[layout_pos + 1].as_str().unwrap()).unwrap(),
            )
            .unwrap();
            let p = actionc::compiler::native65816::prepare_file(
                source,
                artifact["mode"] == "optimized",
                &Default::default(),
            )
            .unwrap();
            let c = p.compile(&options).unwrap();
            let saved =
                Image::from_json(&std::fs::read(artifact["image"].as_str().unwrap()).unwrap())
                    .unwrap();
            assert_eq!(
                c.image.to_json().unwrap(),
                saved.to_json().unwrap(),
                "edge evidence must match the saved compiler artifact"
            );
            let forwarded = support::forwarding::compiled(&p, &c);
            let inventory: Vec<_> = support::control_flow::inventory(&p.mir, &c.machine, &saved)
                .into_iter()
                .filter(|s| contains(&artifact["code_ranges"], number(&s["pc"])))
                .collect();
            control.push(json!({"case": artifact["case"], "mode": artifact["mode"],
                "sites": inventory}));
            let sites = support::word_edge::index(&p.mir, &c.machine, |id| {
                saved
                    .routines
                    .iter()
                    .find(|r| r.id == id.0)
                    .unwrap()
                    .address
            });
            (sites, forwarded)
        } else {
            (Default::default(), Default::default())
        };
        for (vector, input) in case["vectors"].as_array().unwrap().iter().enumerate() {
            eprintln!(
                "{} {} {} vector {vector}",
                case["id"], artifact["compiler"], artifact["mode"]
            );
            let mut result = execute(artifact, case, input, 0, &sites, &forwarded);
            // Both interrupt-mask states must preserve the ABI and produce
            // identical measurements. No IRQ/NMI is injected in this benchmark.
            assert_eq!(
                result,
                execute(artifact, case, input, 4, &sites, &forwarded)
            );
            let object = result.as_object_mut().unwrap();
            for key in [
                "case",
                "compiler",
                "mode",
                "code_bytes",
                "static_stack_check_bytes",
            ] {
                object.insert(key.into(), artifact[key].clone());
            }
            object.insert("vector".into(), json!(vector));
            object.insert("args".into(), input["args"].clone());
            measurements.push(result);
        }
    }
    std::fs::write(
        &destination,
        serde_json::to_string_pretty(
            &json!({"manifest": manifest, "measurements": measurements, "control": control}),
        )
        .unwrap()
            + "\n",
    )
    .unwrap();
    eprintln!(
        "{} paired-mask measurements: {}",
        measurements.len(),
        destination.display()
    );
    let failures: Vec<_> = measurements
        .iter()
        .filter(|m| m["correct"] != true)
        .collect();
    assert!(
        failures.is_empty(),
        "incorrect compiler outputs (all measurements saved): {failures:#?}"
    );
}
