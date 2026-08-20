use actionc::atari_real::AtariReal;
use actionc_vm::{CompilerVm, ExecutionProfile, RUNAD, RunRequest, StopReason, VmRunner};

const LOAD_ADDRESS: u16 = 0x2000;
const INPUT_ADDRESS: u16 = 0x3000;
const CAPTURE_ADDRESS: u16 = 0x0600;
const COMPLETION_MARKER: u16 = 0x06ff;

const CIX: u16 = 0x00f2;
const INBUFF: u16 = 0x00f3;
const FR0: u16 = 0x00d4;
const FR1: u16 = 0x00e0;

const AFP: u16 = 0xd800;
const FSUB: u16 = 0xda60;
const FADD: u16 = 0xda66;
const FMULT: u16 = 0xdadb;
const FDIV: u16 = 0xdb28;

#[derive(Debug, Clone, PartialEq, Eq)]
struct FppCapture {
    workspace: [u8; 18],
    cix: u8,
    registers_and_status: [u8; 4],
}

impl FppCapture {
    fn fr0(&self) -> [u8; 6] {
        self.workspace[..6].try_into().expect("FR0 capture")
    }

    fn fr1(&self) -> [u8; 6] {
        self.workspace[12..].try_into().expect("FR1 capture")
    }
}

fn push_word(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn atari_object(code: &[u8]) -> Vec<u8> {
    let end = LOAD_ADDRESS
        .checked_add(code.len() as u16)
        .and_then(|end| end.checked_sub(1))
        .expect("oracle object fits in memory");
    let mut object = Vec::with_capacity(code.len() + 14);
    object.extend_from_slice(&[0xff, 0xff]);
    push_word(&mut object, LOAD_ADDRESS);
    push_word(&mut object, end);
    object.extend_from_slice(code);
    push_word(&mut object, RUNAD);
    push_word(&mut object, RUNAD + 1);
    push_word(&mut object, LOAD_ADDRESS);
    object
}

fn capture_program(routine: u16) -> Vec<u8> {
    let mut code = vec![0x20, routine as u8, (routine >> 8) as u8]; // JSR routine

    // Preserve the immediately observable A/X/Y/P state before the capture
    // loop changes it. P is stored after PHP/PLA.
    code.extend_from_slice(&[0x8d, 0x20, 0x06]); // STA $0620
    code.extend_from_slice(&[0x8e, 0x21, 0x06]); // STX $0621
    code.extend_from_slice(&[0x8c, 0x22, 0x06]); // STY $0622
    code.extend_from_slice(&[0x08, 0x68, 0x8d, 0x23, 0x06]); // PHP; PLA; STA $0623

    // Capture FR0, the six-byte gap, and FR1 as one contiguous workspace.
    code.extend_from_slice(&[0xa2, 17]); // LDX #17
    let loop_offset = code.len();
    code.extend_from_slice(&[0xbd, 0xd4, 0x00]); // LDA $00D4,X
    code.extend_from_slice(&[0x9d, 0x00, 0x06]); // STA $0600,X
    code.push(0xca); // DEX
    let branch_next = code.len() + 2;
    let displacement = loop_offset as isize - branch_next as isize;
    code.extend_from_slice(&[0x10, displacement as i8 as u8]); // BPL loop

    code.extend_from_slice(&[0xa5, CIX as u8]); // LDA CIX
    code.extend_from_slice(&[0x8d, 0x12, 0x06]); // STA $0612
    code.extend_from_slice(&[0xa9, 0xa5]); // LDA #$A5
    code.extend_from_slice(&[0x8d, 0xff, 0x06]); // STA completion marker

    let halt = LOAD_ADDRESS + code.len() as u16;
    code.extend_from_slice(&[0x4c, halt as u8, (halt >> 8) as u8]); // JMP halt
    code
}

fn call_fpp(routine: u16, setup: impl FnOnce(&mut CompilerVm)) -> FppCapture {
    let code = capture_program(routine);
    let halt = LOAD_ADDRESS + code.len() as u16 - 3;
    let object = atari_object(&code);
    let mut vm = CompilerVm::default();
    vm.load_bundled_altirra_os().expect("load AltirraOS");
    vm.load_atari_object_for_execution(ExecutionProfile::StandaloneObject, &object)
        .expect("load oracle object");
    setup(&mut vm);

    let outcome = VmRunner::new(vm).run(RunRequest {
        max_steps: 50_000,
        stop_after_pc: Some(halt),
        history_len: 8,
    });
    assert_eq!(
        outcome.stop_reason(),
        StopReason::PcReached { pc: halt },
        "unexpected VM stop: {:?}",
        outcome.report
    );
    assert_eq!(outcome.memory().read(COMPLETION_MARKER), 0xa5);

    FppCapture {
        workspace: std::array::from_fn(|offset| {
            outcome.memory().read(CAPTURE_ADDRESS + offset as u16)
        }),
        cix: outcome.memory().read(CAPTURE_ADDRESS + 18),
        registers_and_status: std::array::from_fn(|offset| {
            outcome.memory().read(0x0620 + offset as u16)
        }),
    }
}

fn parse_ascii(text: &str) -> FppCapture {
    call_fpp(AFP, |vm| {
        vm.bus_mut().ram_mut().write_word(INBUFF, INPUT_ADDRESS);
        vm.bus_mut().ram_mut().write(CIX, 0);
        if !text.is_empty() {
            vm.bus_mut()
                .ram_mut()
                .map(INPUT_ADDRESS, text.as_bytes())
                .expect("map AFP input");
        }
        vm.bus_mut()
            .ram_mut()
            .write(INPUT_ADDRESS + text.len() as u16, 0);
    })
}

fn binary(routine: u16, left: [u8; 6], right: [u8; 6]) -> FppCapture {
    call_fpp(routine, |vm| {
        vm.bus_mut().ram_mut().map(FR0, &left).expect("map FR0");
        vm.bus_mut().ram_mut().map(FR1, &right).expect("map FR1");
    })
}

#[test]
fn afp_produces_canonical_six_byte_values() {
    let cases = [
        ("0", [0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
        ("1", [0x40, 0x01, 0x00, 0x00, 0x00, 0x00]),
        ("-1", [0xc0, 0x01, 0x00, 0x00, 0x00, 0x00]),
        (".5", [0x3f, 0x50, 0x00, 0x00, 0x00, 0x00]),
        ("1.25", [0x40, 0x01, 0x25, 0x00, 0x00, 0x00]),
        ("10", [0x40, 0x10, 0x00, 0x00, 0x00, 0x00]),
        ("100", [0x41, 0x01, 0x00, 0x00, 0x00, 0x00]),
        ("1234567890", [0x44, 0x12, 0x34, 0x56, 0x78, 0x90]),
    ];

    for (text, expected) in cases {
        let capture = parse_ascii(text);
        assert_eq!(capture.fr0(), expected, "AFP({text:?})");
        assert_eq!(capture.cix as usize, text.len(), "AFP CIX for {text:?}");
    }
}

#[test]
fn core_fpp_binary_entry_points_use_fr0_and_fr1() {
    let one_and_quarter = parse_ascii("1.25").fr0();
    let two = parse_ascii("2").fr0();
    let cases = [
        (
            FADD,
            [0x40, 0x03, 0x25, 0x00, 0x00, 0x00],
            [0x40, 0x02, 0x00, 0x00, 0x00, 0x00],
            [0x03, 0x00, 0xff, 0xb4],
        ),
        (
            FSUB,
            [0xbf, 0x75, 0x00, 0x00, 0x00, 0x00],
            [0xc0, 0x02, 0x00, 0x00, 0x00, 0x00],
            [0x7e, 0x75, 0x04, 0xb4],
        ),
        (
            FMULT,
            [0x40, 0x02, 0x50, 0x00, 0x00, 0x00],
            [0x01, 0x28, 0x00, 0x00, 0x00, 0x00],
            [0x80, 0x02, 0x05, 0xb4],
        ),
        (
            FDIV,
            [0x3f, 0x62, 0x50, 0x00, 0x00, 0x00],
            [0x00, 0x02, 0x00, 0x00, 0x00, 0x00],
            [0x62, 0x00, 0x7c, 0x36],
        ),
    ];

    for (routine, expected, expected_fr1, expected_registers) in cases {
        let capture = binary(routine, one_and_quarter, two);
        assert_eq!(capture.fr0(), expected, "FPP routine ${routine:04X}");
        assert_eq!(
            capture.fr1(),
            expected_fr1,
            "FR1 after FPP routine ${routine:04X}"
        );
        assert_eq!(
            capture.registers_and_status, expected_registers,
            "A/X/Y/P after FPP routine ${routine:04X}"
        );
    }
}

#[test]
fn afp_rounding_and_range_vectors_are_stable() {
    let cases = [
        ("1.2345678904", [0x40, 0x01, 0x23, 0x45, 0x67, 0x89]),
        ("1.2345678905", [0x40, 0x01, 0x23, 0x45, 0x67, 0x89]),
        ("1.234567895", [0x40, 0x01, 0x23, 0x45, 0x67, 0x89]),
        ("12.34567895", [0x40, 0x12, 0x34, 0x56, 0x78, 0x95]),
        ("12.345678956", [0x40, 0x12, 0x34, 0x56, 0x78, 0x95]),
        ("9.999999999E97", [0x70, 0x99, 0x99, 0x99, 0x99, 0x99]),
        ("1E99", [0x71, 0x10, 0x00, 0x00, 0x00, 0x00]),
        ("1E-98", [0x0f, 0x01, 0x00, 0x00, 0x00, 0x00]),
        ("1E-99", [0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
        ("-0", [0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
    ];

    for (text, expected) in cases {
        assert_eq!(parse_ascii(text).fr0(), expected, "AFP({text:?})");
    }

    let invalid = parse_ascii("X");
    assert_eq!(invalid.fr0(), [0x7f, 0x00, 0x00, 0x00, 0x00, 0x00]);
    assert_eq!(invalid.cix, 0);

    // AFP accepts at most two exponent digits. It consumes `1E10` here and
    // leaves the final zero unconsumed, so compiler literal validation must not
    // silently copy AFP's prefix-accepting behavior.
    let excess_exponent_digit = parse_ascii("1E100");
    assert_eq!(
        excess_exponent_digit.fr0(),
        [0x45, 0x01, 0x00, 0x00, 0x00, 0x00]
    );
    assert_eq!(excess_exponent_digit.cix, 4);
}

#[test]
fn actionc_decimal_codec_matches_the_os_oracle() {
    let cases = [
        "0",
        "-0",
        "1",
        "-1",
        "1.",
        ".5",
        "0.00123",
        "001.2300",
        "12.345678956",
        "1234567890",
        "7E-2",
        "123E-3",
        "1E-98",
        "1E-99",
        "1E98",
        "1E99",
        "9.999999999E97",
        "-98.765432109",
    ];

    for text in cases {
        let compiler = AtariReal::from_decimal(text)
            .unwrap_or_else(|error| panic!("compiler codec rejected {text:?}: {error}"));
        let os = parse_ascii(text);
        assert_eq!(compiler.to_bytes(), os.fr0(), "codec mismatch for {text:?}");
        assert_eq!(os.cix as usize, text.len(), "AFP did not consume {text:?}");
    }
}
