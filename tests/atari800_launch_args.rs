use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "actionc-atari800-launch-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create Atari800 launcher test directory");
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn helper_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tools")
        .join("lib")
        .join("atari800-launch.sh")
}

fn launch_args(
    os_rom: &str,
    cart_rom: &str,
    extra_args: &str,
    no_cart_config: &str,
) -> Vec<String> {
    let output = Command::new("bash")
        .arg("-c")
        .arg(
            "source \"$1\"; actionc_build_atari800_launch_args \"$2\" \"$3\" \"$4\" \"$5\"; printf '%s\\n' \"${ACTIONC_ATARI800_LAUNCH_ARGS[@]}\"",
        )
        .arg("actionc-atari800-launch-test")
        .arg(helper_path())
        .arg(os_rom)
        .arg(cart_rom)
        .arg(extra_args)
        .arg(no_cart_config)
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
        launch_args("/roms/rev02.rom", "/roms/action.rom", "", ""),
        [
            "-xl",
            "-xlxe_rom",
            "/roms/rev02.rom",
            "-cart",
            "/roms/action.rom",
        ]
    );
}

#[test]
fn launcher_without_cart_ignores_saved_atari800_configuration() {
    let args = launch_args("/roms/rev02.rom", "", "-pal", "/tmp/actionc-no-cart.cfg");

    assert_eq!(
        args,
        [
            "-config",
            "/tmp/actionc-no-cart.cfg",
            "-no-autosave-config",
            "-xl",
            "-xlxe_rom",
            "/roms/rev02.rom",
            "-pal",
        ]
    );
    assert!(!args.iter().any(|arg| arg == "-cart"));
}

#[test]
fn sanitized_configuration_preserves_roms_and_clears_all_cartridges() {
    let temp = TestDir::new();
    let source = temp.0.join("source.cfg");
    let target = temp.0.join("no-cart.cfg");
    fs::write(
        &source,
        "ROM_XL/XE_CUSTOM=/roms/rev02.rom\nMACHINE_TYPE=Atari XL/XE\nCARTRIDGE_FILENAME=/roms/action.rom\nCARTRIDGE_TYPE=15\nCARTRIDGE_PIGGYBACK_FILENAME=/roms/second.car\nCARTRIDGE_PIGGYBACK_TYPE=1\n",
    )
    .expect("write source Atari800 configuration");

    let output = Command::new("bash")
        .arg("-c")
        .arg("source \"$1\"; actionc_write_no_cart_config \"$2\" \"$3\"")
        .arg("actionc-atari800-config-test")
        .arg(helper_path())
        .arg(&source)
        .arg(&target)
        .output()
        .expect("sanitize Atari800 configuration");
    assert!(
        output.status.success(),
        "configuration sanitizer failed\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        fs::read_to_string(target).expect("read sanitized Atari800 configuration"),
        "ROM_XL/XE_CUSTOM=/roms/rev02.rom\nMACHINE_TYPE=Atari XL/XE\nCARTRIDGE_FILENAME=\nCARTRIDGE_TYPE=0\nCARTRIDGE_PIGGYBACK_FILENAME=\nCARTRIDGE_PIGGYBACK_TYPE=0\n"
    );
}
