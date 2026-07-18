use std::path::Path;
use std::process::Command;

fn launch_args(os_rom: &str, cart_rom: &str, extra_args: &str) -> Vec<String> {
    let helper = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tools")
        .join("lib")
        .join("atari800-launch.sh");
    let output = Command::new("bash")
        .arg("-c")
        .arg(
            "source \"$1\"; actionc_build_atari800_launch_args \"$2\" \"$3\" \"$4\"; printf '%s\\n' \"${ACTIONC_ATARI800_LAUNCH_ARGS[@]}\"",
        )
        .arg("actionc-atari800-launch-test")
        .arg(helper)
        .arg(os_rom)
        .arg(cart_rom)
        .arg(extra_args)
        .output()
        .expect("run Atari800 launch argument helper");
    assert!(
        output.status.success(),
        "argument helper failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("argument helper output is UTF-8")
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn launcher_forces_xl_and_attaches_explicit_cartridge() {
    assert_eq!(
        launch_args("/roms/rev02.rom", "/roms/action.rom", ""),
        [
            "-xl",
            "-xlxe_rom",
            "/roms/rev02.rom",
            "-xl-rev",
            "custom",
            "-cart",
            "/roms/action.rom",
        ]
    );
}

#[test]
fn launcher_without_cart_ignores_saved_atari800_configuration() {
    let args = launch_args("/roms/rev02.rom", "", "-pal");

    assert_eq!(
        args,
        [
            "-config",
            "/dev/null",
            "-no-autosave-config",
            "-xl",
            "-xlxe_rom",
            "/roms/rev02.rom",
            "-xl-rev",
            "custom",
            "-pal",
        ]
    );
    assert!(!args.iter().any(|arg| arg == "-cart"));
}
