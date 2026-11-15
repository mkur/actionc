// Small independent execution witness for the Position flag-summary audit.
// Usage: probe SOURCE.act OUTPUT_DIRECTORY. See the audit report for linking.
use actionc::{compiler::Runtime, mir6502::{self, Mir6502Config}};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let destination = std::path::Path::new(&args[2]);
    std::fs::create_dir_all(destination).unwrap();
    let loaded = actionc::includes::load_program_with_expanded_source(&args[1]).unwrap();
    let model = actionc::semantic::analyze_with_options(&loaded.program, actionc::semantic::SemanticOptions::modern()).unwrap();
    let nir = actionc::nir::optimize_program(&actionc::nir::lower_program(&actionc::semantic::ir::lower_program(&loaded.program, &model))).unwrap();
    let mut results = Vec::new();
    for (name, config) in [("optimized", Mir6502Config::optimized()), ("default", Mir6502Config::default())] {
        let out = mir6502::generate_output_with_config_and_runtime(&nir, 0x3000, &config, Runtime::ActionCart).unwrap();
        let mut listing = String::new();
        for r in &out.map.routine_ranges {
            listing.push_str(&format!("; {}\n{}\n\n", r.name, actionc::codegen::format_listing_with_origin(
                &out.bytes[(r.start-out.origin) as usize..(r.end-out.origin) as usize], r.start)));
        }
        std::fs::write(destination.join(format!("position-{name}.asm")), listing).unwrap();
        let entry = out.routine_addresses.iter().find(|r| r.name == "Check").unwrap().address;
        // No runtime services are used by this control; execute its RAM image.
        let mut vm = actionc_vm::CompilerVm::default();
        vm.bus_mut().ram_mut().map(out.origin, &out.bytes).unwrap();
        vm.bus_mut().ram_mut().map(0x700, &[0x20, entry as u8, (entry>>8) as u8, 0xea]).unwrap();
        vm.set_pc(0x700);
        let initial_sp = vm.cpu().registers().sp;
        for _ in 0..100 {
            if vm.cpu().registers().pc == 0x703 { break; }
            vm.step_cpu().unwrap();
        }
        assert_eq!(vm.cpu().registers().pc, 0x703);
        assert_eq!(vm.cpu().registers().sp, initial_sp);
        let (observed, kx, ky) = (vm.bus_mut().read(0xa0), vm.bus_mut().read(0xff), vm.bus_mut().read(0xfe));
        assert_eq!((kx, ky), (0, 128));
        results.push(format!("{{\"config\":\"{name}\",\"expected\":1,\"observed\":{observed},\"kx\":{kx},\"ky\":{ky},\"stack_balanced\":true}}"));
        println!("{name}: expected=1 observed={observed}; kx={kx} ky={ky}");
    }
    std::fs::write(destination.join("position-control-results.json"), format!("{{\"cases\":[{}]}}\n", results.join(","))).unwrap();
}
