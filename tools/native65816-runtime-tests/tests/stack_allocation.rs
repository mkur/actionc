mod support;
use actionc_vm::native65816::Inputs;
use support::*;

// Inputs are supplied by the host after linking so optimized measurements do
// not collapse to constant programs. Each result is independently specified.
#[test]
fn representative_stack_pressure_executes_in_both_modes() {
    for (name, source, expected, stack_limits) in [
        (
            "scalar_chain",
            "CARD input,result CARD FUNC Chain(CARD n) \
             RETURN(n+1+2+3+4+5+6+7+8+9+10+11+12+13+14+15+16) \
             PROC Main() result=Chain(input) RETURN",
            149,
            [22, 22],
        ),
        (
            "loop_rotation",
            "CARD input,result CARD FUNC Rotate(CARD n) CARD i,a,b,c \
             a=n b=n+1 FOR i=1 TO 8 DO c=a a=b b=c+1 OD RETURN(a+b) \
             PROC Main() result=Rotate(input) RETURN",
            35,
            [34, 42],
        ),
        (
            "recursive_sum",
            "CARD input,result CARD FUNC Sum(CARD n) \
             IF n=0 THEN RETURN(0) FI RETURN(n+Sum(n-1)) \
             PROC Main() result=Sum(input) RETURN",
            91,
            [206, 206],
        ),
        (
            "wide_indirect",
            "CARD input LONGCARD result \
             LONGCARD FUNC Add(LONGCARD n) RETURN(n+LONGCARD($10001)) \
             LONGCARD FUNC Work(CARD n) LONGCARD FUNC POINTER cb(LONGCARD x) \
             cb=@Add RETURN(LONGCARD(n)+cb(LONGCARD(n) LSH 16)) \
             PROC Main() result=Work(input) RETURN",
            0xe000e,
            [70, 50],
        ),
    ] {
        for optimize in [false, true] {
            let image = compile(source, optimize);
            let caller = caller(image.entry);
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller, mask);
                let input = image.data.iter().find(|d| d.name == "input").unwrap();
                h.bus.ram[input.address as usize..input.address as usize + 2]
                    .copy_from_slice(&13u16.to_le_bytes());
                let mut lowest_s = h.cpu.registers().s;
                for _ in 0..2_000_000 {
                    if h.cpu.is_stopped() {
                        break;
                    }
                    h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                    lowest_s = lowest_s.min(h.cpu.registers().s);
                }
                assert!(h.cpu.is_stopped(), "{name}: execution budget");
                h.guards(mask);
                assert!(
                    0x5ff0 - lowest_s <= stack_limits[usize::from(optimize)],
                    "{name}: stack use {}",
                    0x5ff0 - lowest_s
                );
                let result = image.data.iter().find(|d| d.name == "result").unwrap();
                assert_eq!(h.bus.value(result.address, result.size as usize), expected);
                if mask == 0 {
                    eprintln!(
                        "{name} optimize={optimize}: {} code bytes, {} VM cycles, {} observed stack bytes; frames {:?}",
                        image.routines.iter().map(|r| r.size).sum::<u32>(),
                        h.cpu.cycles(),
                        0x5ff0 - lowest_s,
                        image
                            .routines
                            .iter()
                            .map(|r| (&r.name, r.fixed_frame, r.spill_bytes, r.local_stack_peak))
                            .collect::<Vec<_>>()
                    );
                }
            }
        }
    }
}

#[test]
fn long_scalar_sequence_fits_without_raising_the_frame_limit() {
    let statements = (1..=160).map(|i| format!("n==+{i} ")).collect::<String>();
    let source = format!(
        "CARD result CARD FUNC Chain(CARD n) {statements} RETURN(n) PROC Main() result=Chain(13) RETURN"
    );
    for optimize in [false, true] {
        let image = compile(&source, optimize);
        let chain = image.routines.iter().find(|r| r.name == "Chain").unwrap();
        assert!(chain.fixed_frame <= 10, "{}", chain.fixed_frame);
        if !optimize {
            assert!(chain.temporaries.len() > 160);
        }
        let mut h = Harness::new(&image, &caller(image.entry), 0);
        h.run();
        h.guards(0);
        assert_eq!(h.global(&image, "result", 2), 12893);
    }
}

#[test]
fn live_values_survive_direct_and_indirect_full_scratch_clobbers() {
    use actionc::{
        mir65816::{
            abi,
            image::{AssemblyImport, Image},
        },
        nir::runtime_symbol_id,
    };
    let source = r#"
MODULE TEST
PUBLIC EXTERNAL LONGCARD FUNC Smash(LONGCARD ignored)
LONGCARD result
LONGCARD FUNC Work(LONGCARD n)
  LONGCARD FUNC POINTER cb(LONGCARD x)
  cb=@Smash
RETURN((n+LONGCARD($10203))+Smash(n)+cb(n))
PROC Main() result=Work(LONGCARD($55667788)) RETURN
ENDMODULE
"#;
    // This independent ABI callee destroys every call-clobbered DP byte and
    // CPU register, then supplies the specified A/X result. No stack pushes.
    let smash = assemble(
        r#"
        sep #$20
        .a8
        ldx #63
        lda #$a7
    again:
        sta 0,x
        dex
        bpl again
        rep #$20
        .a16
        ldy #$dead
        ldx #$1122
        lda #$3344
        rtl
    "#,
        0x041000,
    );
    for optimize in [false, true] {
        let prepared = prepare(source, optimize);
        let symbol = runtime_symbol_id("TEST.Smash");
        let imported = prepared
            .mir
            .routines
            .iter()
            .find(|r| r.entry.external_symbol == Some(symbol))
            .unwrap();
        let mut options = layout();
        options.imports.push(AssemblyImport {
            symbol: symbol.0,
            signature: imported.signature.0,
            abi: abi::generated::ABI_NAME.into(),
            address: 0x041000,
            size: smash.len() as u32,
            stack_peak: 0,
            checks_stack: true,
            irq_effect: Default::default(),
        });
        let compiled = prepared.compile(&options).unwrap();
        let image = Image::from_json(&compiled.image.to_json().unwrap()).unwrap();
        for mask in [0, 4] {
            let mut h = Harness::new(&image, &caller(image.entry), mask);
            h.bus.map(0x041000, &smash, false);
            h.run();
            h.guards(mask);
            assert_eq!(
                h.global(&image, "result", 4),
                0x55667788 + 0x10203 + 2 * 0x11223344
            );
            assert!((0x2000..0x2040).all(|a| h.bus.writes.contains(&(a, 0xa7))));
        }
    }
}
