//! Observe existing bytes only. Inventory claims never control CPU execution.
mod support;
use actionc::mir65816::image::Image;
use actionc_vm::native65816::{Access, Inputs, Machine, Registers};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf};
use support::Bus;

fn n(v: &Value) -> u32 {
    v.as_u64().unwrap().try_into().unwrap()
}
fn bytes(v: &Value) -> Vec<u8> {
    v.as_array().unwrap().iter().map(|v| n(v) as u8).collect()
}

#[test]
#[ignore = "requires A816_MOVEMENT_INVENTORY and A816_COMPARISON_MANIFEST"]
fn observe_every_reload_claim_on_saved_raw_and_optimized_images() {
    let inventory: Value = serde_json::from_slice(
        &std::fs::read(std::env::var_os("A816_MOVEMENT_INVENTORY").unwrap()).unwrap(),
    )
    .unwrap();
    let path = PathBuf::from(std::env::var_os("A816_COMPARISON_MANIFEST").unwrap());
    let baseline: Value =
        serde_json::from_slice(&std::fs::read(path.parent().unwrap().join("debug.json")).unwrap())
            .unwrap();
    let manifest = &baseline["manifest"];
    assert_eq!(
        *manifest,
        serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap()
    );
    assert_eq!(
        baseline,
        serde_json::from_slice::<Value>(
            &std::fs::read(path.parent().unwrap().join("release.json")).unwrap()
        )
        .unwrap()
    );
    let mut observations = vec![];
    for artifact in manifest["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["compiler"] == "actionc")
    {
        let build = inventory["builds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["case"] == artifact["case"] && b["mode"] == artifact["mode"])
            .unwrap();
        let claims: BTreeMap<_, _> = build["routines"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|r| r["reloads"].as_array().unwrap())
            .filter(|r| r.get("conditional_saving").is_some())
            .map(|r| (n(&r["pc"]), r))
            .collect();
        let case = manifest["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == artifact["case"])
            .unwrap();
        let image =
            Image::from_json(&std::fs::read(artifact["image"].as_str().unwrap()).unwrap()).unwrap();
        let command = artifact["commands"][0].as_array().unwrap();
        let at = command.iter().position(|v| v == "--layout").unwrap();
        let options =
            serde_json::from_slice(&std::fs::read(command[at + 1].as_str().unwrap()).unwrap())
                .unwrap();
        let compiled = actionc::compiler::native65816::prepare_file(
            command.last().unwrap().as_str().unwrap(),
            artifact["mode"] == "optimized",
            &Default::default(),
        )
        .unwrap()
        .compile(&options)
        .unwrap();
        assert_eq!(
            compiled.image.to_json().unwrap(),
            image.to_json().unwrap(),
            "observer must execute the measured image"
        );
        for (vector, input) in case["vectors"].as_array().unwrap().iter().enumerate() {
            let expected = baseline["measurements"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| {
                    r["case"] == artifact["case"]
                        && r["mode"] == artifact["mode"]
                        && r["compiler"] == "actionc"
                        && n(&r["vector"]) == vector as u32
                })
                .unwrap();
            for mask in [0, 4] {
                let mut bus = Bus::new();
                bus.load(&image);
                bus.map(0x040000, &[0xdb, 0xea], false);
                bus.map(0x048000, &[0xdb, 0xea], false);
                bus.map(
                    0x2000,
                    &(0..256)
                        .map(|i| (i as u8).wrapping_mul(37).wrapping_add(0x93))
                        .collect::<Vec<_>>(),
                    true,
                );
                bus.ram[0x2044..0x2046].copy_from_slice(&0x401au16.to_le_bytes());
                bus.ram[0x2046..0x2048].copy_from_slice(&0x6000u16.to_le_bytes());
                bus.map(0x4000, &[0xa5; 0x2000], true);
                let entry = n(&artifact["entry"]);
                let entry_s = 0x5fe0u16;
                bus.ram[0x5fe1..0x5fe4].copy_from_slice(&[0xff, 0xff, 4]);
                for (arg, v) in artifact["arguments"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .zip(input["args"].as_array().unwrap())
                {
                    let at = 0x5fe4 + n(&arg["offset"]) as usize;
                    let size = n(&arg["size"]) as usize;
                    bus.ram[at..at + size].copy_from_slice(&n(v).to_le_bytes()[..size]);
                }
                for region in input["memory"].as_array().unwrap() {
                    let mut guarded = vec![0xa7];
                    guarded.extend(bytes(&region["bytes"]));
                    guarded.extend([0x5d, 0xe3]);
                    bus.map(n(&region["address"]) - 1, &guarded, true);
                }
                let mut cpu = Machine::start_at(Registers {
                    a: 0xabcd,
                    x: 0x5678,
                    y: 0x9abc,
                    s: entry_s,
                    d: 0x2000,
                    dbr: 0,
                    pbr: (entry >> 16) as u8,
                    pc: entry as u16,
                    p: mask,
                    emulation_mode: false,
                });
                let mut counts = BTreeMap::<u32, u64>::new();
                let mut reached = BTreeMap::<u32, u64>::new();
                let mut values = BTreeMap::<u32, Vec<(u16, u8, u16)>>::new();
                let mut pending = None;
                for _ in 0..2_000_000 {
                    if cpu.is_instruction_boundary() {
                        if let Some((before, cycles, at)) = pending.take() {
                            let mut wanted: Registers = before;
                            wanted.pc = wanted.pc.wrapping_add(2);
                            assert_eq!(cpu.registers(), wanted, "reload changed full CPU state");
                            assert_eq!(cpu.cycles() - cycles, 5);
                            assert_eq!(
                                bus.trace
                                    .iter()
                                    .map(|&(_, a, v)| (a, v))
                                    .collect::<Vec<_>>(),
                                [(at, Access::Read), (at + 1, Access::Read)]
                            );
                            bus.watched.clear();
                            bus.trace.clear();
                        }
                        let pc = cpu.pc();
                        if pc == 0x040000 {
                            break;
                        }
                        assert_ne!(pc, 0x048000);
                        *counts.entry(pc).or_default() += 1;
                        if let Some(claim) = claims.get(&pc) {
                            let before = cpu.registers();
                            assert_eq!(before.p & 0x30, 0);
                            let slot = n(&claim["slot"]);
                            assert_eq!(bus.ram[pc as usize..pc as usize + 2], [0xa3, slot as u8]);
                            for pair in claim["window"].as_array().unwrap() {
                                let at = n(&pair[0]) as usize;
                                let hex = pair[1].as_str().unwrap();
                                let code: Vec<_> = (0..hex.len())
                                    .step_by(2)
                                    .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                                    .collect();
                                assert_eq!(&bus.ram[at..at + code.len()], code);
                            }
                            let at = u32::from(before.s) + slot;
                            let value = bus.value(at, 2) as u16;
                            assert_eq!(before.a, value);
                            assert_eq!(
                                before.p & 0x82,
                                if value == 0 {
                                    2
                                } else if value & 0x8000 != 0 {
                                    0x80
                                } else {
                                    0
                                }
                            );
                            *reached.entry(pc).or_default() += 1;
                            let sample = (value, before.p, before.s);
                            let samples = values.entry(pc).or_default();
                            if !samples.contains(&sample) {
                                samples.push(sample);
                            }
                            bus.watched = [at, at + 1].into();
                            bus.trace.clear();
                            pending = Some((before, cpu.cycles(), at));
                        }
                    }
                    cpu.tick(&mut bus, Inputs::default()).unwrap();
                }
                assert_eq!(cpu.pc(), 0x040000, "execution budget exhausted");
                assert_eq!(cpu.cycles(), expected["cycles"].as_u64().unwrap());
                assert_eq!(
                    serde_json::to_value(&counts).unwrap(),
                    expected["instruction_sites"]
                );
                let returned = cpu.registers();
                assert_eq!(returned.s, entry_s + 3);
                assert_eq!(returned.p & 4, mask);
                let result = match n(&case["returns"]) {
                    0 => Value::Null,
                    2 => json!(returned.a),
                    4 => json!(u32::from(returned.a) | (u32::from(returned.x) << 16)),
                    _ => panic!(),
                };
                assert_eq!(result, input["result"]);
                for region in input["memory"].as_array().unwrap() {
                    let expected = input["after"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|r| r["address"] == region["address"])
                        .unwrap_or(region);
                    let at = n(&expected["address"]) as usize;
                    let data = bytes(&expected["bytes"]);
                    assert_eq!(&bus.ram[at..at + data.len()], data);
                }
                for (&pc, claim) in &claims {
                    let count = claim["executions"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|r| n(&r["vector"]) == vector as u32)
                        .unwrap();
                    assert_eq!(
                        reached.get(&pc).copied().unwrap_or(0),
                        count["count"].as_u64().unwrap()
                    );
                }
                observations.push(json!({"case":artifact["case"],"mode":artifact["mode"],"vector":vector,"incoming_i":mask,"cycles":cpu.cycles(),"claim_counts":reached,"observed_a_p_s":values}));
            }
        }
    }
    assert_eq!(observations.len(), 264);
    let dir = PathBuf::from(std::env::var_os("A816_QUALIFICATION_DIR").unwrap());
    std::fs::write(
        dir.join("movement-observations.json"),
        serde_json::to_vec_pretty(&observations).unwrap(),
    )
    .unwrap();
}
