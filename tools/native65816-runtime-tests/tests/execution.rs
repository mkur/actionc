mod support;
use support::*;

#[test]
fn emitted_integer_arithmetic_casts_branches_and_recursion_execute() {
    let source = r#"
LONGINT signedResult
LONGCARD wideResult
CARD sumResult,changed
BYTE branchResult
LONGINT FUNC Add(INT first LONGINT second)
RETURN(LONGINT(first)+second)
CARD FUNC Sum(CARD n)
  IF n=0 THEN RETURN(0) FI
RETURN(n+Sum(n-1))
CARD FUNC Change(CARD n)
  n==+1
RETURN(n)
PROC Main()
  signedResult=Add(-32768,-100)
  wideResult=LONGCARD($1234FFFF)+LONGCARD($10001)
  sumResult=Sum(12)
  changed=Change($FFFF)
  IF signedResult<0 AND wideResult>LONGCARD($1234FFFF) THEN branchResult=1 ELSE branchResult=2 FI
RETURN
"#;
    for optimize in [false, true] {
        let image = compile(source, optimize);
        for mask in [0, 4] {
            let mut h = Harness::new(&image, &caller(image.entry), mask);
            h.run();
            h.guards(mask);
            assert_eq!(h.global(&image, "signedResult", 4), (-32868i32) as u32);
            assert_eq!(h.global(&image, "wideResult", 4), 0x12360000);
            assert_eq!(h.global(&image, "sumResult", 2), 78);
            assert_eq!(h.global(&image, "changed", 2), 0);
            assert_eq!(h.global(&image, "branchResult", 1), 1);
        }
    }
}

#[test]
fn optimized_loop_edges_and_parallel_values_execute() {
    let source = "CARD result PROC Main() CARD i,a,b,c a=1 b=2 FOR i=1 TO 8 DO c=a a=b b=c+1 OD result=a+b RETURN";
    for optimize in [false, true] {
        let image = compile(source, optimize);
        let mut h = Harness::new(&image, &caller(image.entry), 4);
        h.run();
        h.guards(4);
        assert_eq!(h.global(&image, "result", 2), 11);
    }
}

#[test]
fn indexed_objects_live_local_addresses_and_code_pointers_retain_their_banks() {
    let source = r#"
TYPE Triple=[BYTE first,second,third]
Triple ARRAY triples(2)
CARD ARRAY values=[1 $FFFF 3]
CARD localResult,arrayResult
BYTE fieldResult
PROC POINTER saved
PROC Empty() RETURN
PROC Save(PROC POINTER cb) saved=cb RETURN
CARD FUNC Read(CARD POINTER p) RETURN(p^)
CARD FUNC Local(CARD n)
  CARD ARRAY slots(2)
  slots(1)=n
RETURN(Read(@slots(1)))
PROC Main()
  SIZE i
  i=1
  triples(i).second=7
  fieldResult=triples(i).second
  arrayResult=values(i)
  localResult=Local($A5C3)
  Save(@Empty)
RETURN
"#;
    for optimize in [false, true] {
        let prepared = prepare(source, optimize);
        let machine = actionc::mir65816::emit::materialize(&prepared.mir).unwrap();
        let mut options = layout();
        options.code_origin = 0x020000 - machine.routines[0].code.bytes.len() as u32 - 1;
        let compiled = prepared.compile(&options).unwrap();
        let image =
            actionc::mir65816::image::Image::from_json(&compiled.image.to_json().unwrap()).unwrap();
        assert!(image.routines.iter().any(|r| r.address >> 16 == 1));
        assert!(image.routines.iter().any(|r| r.address >> 16 == 2));
        let mut h = Harness::new(&image, &caller(image.entry), 4);
        h.run();
        h.guards(4);
        assert_eq!(h.global(&image, "fieldResult", 1), 7);
        assert_eq!(h.global(&image, "arrayResult", 2), 65535);
        assert_eq!(h.global(&image, "localResult", 2), 0xa5c3);
        assert_eq!(
            h.global(&image, "saved", 3),
            image
                .routines
                .iter()
                .find(|r| r.name == "Empty")
                .unwrap()
                .address
        );
    }
}

#[test]
fn pointer_memory_and_24_bit_address_results_cross_a_data_bank() {
    let source = r#"
ADDRESS location,addressResult
SIZE sizeResult
CARD readResult
CARD POINTER ptr,pointerResult
ADDRESS FUNC EchoAddress(ADDRESS value) RETURN(value)
SIZE FUNC EchoSize(SIZE value) RETURN(value)
CARD POINTER FUNC EchoPointer(CARD POINTER value) RETURN(value)
PROC Main()
  location=$12FFFF
  ptr=CARD POINTER(location)
  readResult=ptr^
  ptr^=$3456
  addressResult=EchoAddress(location)
  sizeResult=EchoSize($ABCDEF)
  pointerResult=EchoPointer(ptr)
RETURN
"#;
    for optimize in [false, true] {
        let image = compile(source, optimize);
        let mut h = Harness::new(&image, &caller(image.entry), 4);
        h.bus.map(0x12ffff, &[0xcd, 0xab], true);
        h.run();
        h.guards(4);
        assert_eq!(h.global(&image, "readResult", 2), 0xabcd);
        assert_eq!(h.bus.value(0x12ffff, 2), 0x3456);
        assert_eq!(h.global(&image, "addressResult", 3), 0x12ffff);
        assert_eq!(h.global(&image, "sizeResult", 3), 0xabcdef);
        assert_eq!(h.global(&image, "pointerResult", 3), 0x12ffff);
    }
}

#[test]
fn volatile_byte_accesses_are_exact_and_ordered_with_an_unmapped_neighbor() {
    let source =
        "VOLATILE BYTE io=$D000 BYTE first,second PROC Main() io=1 first=io io=2 second=io RETURN";
    for optimize in [false, true] {
        let image = compile(source, optimize);
        let mut h = Harness::new(&image, &caller(image.entry), 0);
        h.bus.map(0xd000, &[0], true);
        h.run();
        h.guards(0);
        assert_eq!(
            h.bus
                .writes
                .iter()
                .filter(|&&(a, _)| a == 0xd000)
                .copied()
                .collect::<Vec<_>>(),
            [(0xd000, 1), (0xd000, 2)]
        );
        assert_eq!(h.bus.reads.iter().filter(|&&a| a == 0xd000).count(), 2);
        assert_eq!(h.global(&image, "first", 1), 1);
        assert_eq!(h.global(&image, "second", 1), 2);
    }
}
