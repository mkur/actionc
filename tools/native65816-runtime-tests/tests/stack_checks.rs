mod support;
use actionc::mir65816::image::Image;
use actionc_vm::native65816::Inputs;
use support::*;

const SOURCE: &str = r#"
SIZE left,right,product
LONGINT dividend,divisor,quotient,remainder
CARD depth
CARD FUNC Recurse(CARD count)
  IF count=0 THEN RETURN(1) FI
RETURN(Recurse(count-1)+count)
PROC Main()
  product=left*right
  quotient=dividend/divisor
  remainder=dividend MOD divisor
  depth=Recurse(5)
RETURN
"#;
const FAULT: u32 = 0x049000;

fn image(optimize: bool, checks: bool) -> Image {
    let mut options = layout();
    options.stack_checks = checks;
    options.arithmetic_fault = Some(FAULT);
    let compiled = prepare(SOURCE, optimize).compile(&options).unwrap();
    Image::from_json(&compiled.image.to_json().unwrap()).unwrap()
}

fn set(h: &mut Harness, image: &Image, name: &str, bytes: usize, value: u32) {
    let address = image
        .data
        .iter()
        .find(|d| {
            d.name.eq_ignore_ascii_case(name)
                || d.name
                    .to_ascii_uppercase()
                    .contains(&format!("_{}_", name.to_ascii_uppercase()))
        })
        .unwrap()
        .address as usize;
    h.bus.ram[address..address + bytes].copy_from_slice(&value.to_le_bytes()[..bytes]);
}

#[test]
fn optional_checks_preserve_recursive_frames_size_products_and_arithmetic_helpers() {
    for optimize in [false, true] {
        for checks in [true, false] {
            let image = image(optimize, checks);
            let caller = caller(image.entry);
            for mask in [0, 4] {
                for (left, right) in [(511u32, 128u32), (0x800000, 3), (0xffffff, 2)] {
                    let mut h = Harness::new(&image, &caller, mask);
                    set(&mut h, &image, "left", 3, left);
                    set(&mut h, &image, "right", 3, right);
                    set(&mut h, &image, "dividend", 4, (-123456i32) as u32);
                    set(&mut h, &image, "divisor", 4, 7);
                    h.run();
                    h.guards(mask);
                    assert_eq!(
                        h.global(&image, "product", 3),
                        left.wrapping_mul(right) & 0xffffff
                    );
                    assert_eq!(h.global(&image, "quotient", 4), (-123456i32 / 7) as u32);
                    assert_eq!(h.global(&image, "remainder", 4), (-123456i32 % 7) as u32);
                    assert_eq!(h.global(&image, "depth", 2), 16);
                }
            }
        }
    }
}

#[test]
fn disabling_stack_checks_keeps_divide_by_zero_faults() {
    for optimize in [false, true] {
        let image = image(optimize, false);
        let mut h = Harness::new(&image, &caller(image.entry), 4);
        h.bus.map(FAULT, &[0xdb, 0xea], false);
        assert!(
            h.cpu
                .run_until(
                    &mut h.bus,
                    100_000,
                    |_| Inputs::default(),
                    |cpu| cpu.is_instruction_boundary() && cpu.pc() == FAULT
                )
                .unwrap()
        );
        assert_eq!(&h.bus.ram[0x2000..0x2080], h.caller_workspace);
        assert_eq!(&h.bus.ram[0x20c0..0x2100], h.domain_tail);
    }
}
