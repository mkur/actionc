mod support;
use actionc_vm::native65816::Inputs;
use support::*;

// Inputs are supplied by the host after linking so optimized measurements do
// not collapse to constant programs. Each result is independently specified.
#[test]
fn representative_stack_pressure_executes_in_both_modes() {
    for (name, source, expected) in [
        (
            "scalar_chain",
            "CARD input,result CARD FUNC Chain(CARD n) \
             RETURN(n+1+2+3+4+5+6+7+8+9+10+11+12+13+14+15+16) \
             PROC Main() result=Chain(input) RETURN",
            149,
        ),
        (
            "loop_rotation",
            "CARD input,result CARD FUNC Rotate(CARD n) CARD i,a,b,c \
             a=n b=n+1 FOR i=1 TO 8 DO c=a a=b b=c+1 OD RETURN(a+b) \
             PROC Main() result=Rotate(input) RETURN",
            35,
        ),
        (
            "recursive_sum",
            "CARD input,result CARD FUNC Sum(CARD n) \
             IF n=0 THEN RETURN(0) FI RETURN(n+Sum(n-1)) \
             PROC Main() result=Sum(input) RETURN",
            91,
        ),
        (
            "wide_indirect",
            "CARD input LONGCARD result \
             LONGCARD FUNC Add(LONGCARD n) RETURN(n+LONGCARD($10001)) \
             LONGCARD FUNC Work(CARD n) LONGCARD FUNC POINTER cb(LONGCARD x) \
             cb=@Add RETURN(LONGCARD(n)+cb(LONGCARD(n) LSH 16)) \
             PROC Main() result=Work(input) RETURN",
            0xe000e,
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
