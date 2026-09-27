mod support;
use support::*;
use actionc_vm::native65816::Inputs;

const SOURCE: &str = r#"
MODULE SAMPLE
USE CSTRING.IMPL AS STR
BYTE ARRAY initialized(8)=c"Hi\n"
BYTE ARRAY action="Hi"
BYTE ARRAY buffer(8)
TYPE Holder=[CSTRING text BYTE tag]
Holder item
CSTRING text,found
CSTRING FUNC POINTER indirect(CSTRING input)
SIZE copied,appended,limited,longLength,extent
INT comparison
BYTE status

CSTRING FUNC Identity(CSTRING input)
RETURN(input)
CSTRING FUNC Literal()
RETURN(c"ab\0cd")

PROC Main()
  status=0
  item.text=Literal()
  item.tag=37
  indirect=@Identity
  text=indirect(item.text)
  IF text(0)#'a THEN RETURN FI
  IF text^#'a THEN RETURN FI
  IF text(2)#0 THEN RETURN FI
  IF text(3)#'c THEN RETURN FI
  IF item.tag#37 THEN RETURN FI
  IF SIZEOF(CSTRING)#3 THEN RETURN FI
  IF SIZEOF(text)#3 THEN RETURN FI
  extent=SIZEOF(initialized)
  copied=STR.strlcpy(buffer,c"Hi",SIZEOF(buffer))
  appended=STR.strlcat(buffer,c" there!",SIZEOF(buffer))
  IF copied#2 THEN RETURN FI
  IF appended#9 THEN RETURN FI
  IF STR.strcmp(CSTRING(buffer),c"Hi ther")#0 THEN RETURN FI
  IF STR.strcmp(c"",c"")#0 THEN RETURN FI
  IF STR.strcmp(c"a",c"ab")>=0 THEN RETURN FI
  IF STR.strcmp(c"\xff",c"\x80")<=0 THEN RETURN FI
  IF STR.strncmp(c"abc",c"abd",2)#0 THEN RETURN FI
  IF STR.strncmp(c"abc",c"abd",3)>=0 THEN RETURN FI
  IF STR.strncmp(CSTRING(0),CSTRING(0),0)#0 THEN RETURN FI
  IF STR.strnlen(BYTE POINTER(0),0)#0 THEN RETURN FI
  IF STR.strchr(text,'z)#CSTRING(0) THEN RETURN FI
  found=STR.strchr(text,0)
  IF (ADDRESS(found)-ADDRESS(text))#2 THEN RETURN FI
  found=STR.strchr(text,'b)
  IF found^#'b THEN RETURN FI
  IF STR.strlcpy(BYTE POINTER(0),c"abc",0)#3 THEN RETURN FI
  IF STR.strlcat(BYTE POINTER(0),c"abc",0)#3 THEN RETURN FI
  IF STR.strlcpy(buffer,c"abc",1)#3 THEN RETURN FI
  IF buffer(0)#0 THEN RETURN FI
  IF STR.strlcat(buffer,c"abc",1)#3 THEN RETURN FI
  IF buffer(0)#0 THEN RETURN FI
  IF STR.strlcpy(buffer,c"abcdefg",8)#7 THEN RETURN FI
  IF STR.strlcat(buffer,c"",8)#7 THEN RETURN FI
  IF STR.strlcat(buffer,c"z",8)#8 THEN RETURN FI
  limited=STR.strnlen(BYTE POINTER($21FFFE),3)
  IF STR.strlcat(BYTE POINTER($21FFFE),c"abc",3)#6 THEN RETURN FI
  IF STR.strncmp(CSTRING(BYTE POINTER($21FFFE)),c"xyz!",3)#0 THEN RETURN FI
  IF STR.strlcpy(BYTE POINTER($32FFFF),c"12345",4)#5 THEN RETURN FI
  status=1
RETURN
ENDMODULE
"#;

#[test]
fn literals_and_library_execute_with_exact_bounds_and_independent_invocations() {
    for optimize in [false, true] {
        let image=compile(SOURCE,optimize);
        for mask in [0,4] {
            let mut h=Harness::new(&image,&caller(image.entry),mask);
            h.bus.map(0x21fffd,b"!xyz?",true);
            h.bus.map(0x32fffe,b"!abcde?",true);
            h.bus.watched.extend(0x21fffd..0x220002);
            h.run(); h.guards(mask);
            assert_eq!(h.global(&image,"status",1),1);
            assert_eq!(h.global(&image,"extent",3),8);
            assert_eq!(h.global(&image,"limited",3),3);
            let at=support::context::symbol(&image,"initialized") as usize;
            assert_eq!(&h.bus.ram[at..at+8],b"Hi\n\0\0\0\0\0");
            let at=support::context::symbol(&image,"action") as usize;
            assert_eq!(&h.bus.ram[at..at+3],b"\x02Hi");
            assert_eq!(&h.bus.ram[0x21fffd..0x220002],b"!xyz?");
            assert_eq!(&h.bus.ram[0x32fffe..0x330005],b"!123\0e?");
            assert!(h.bus.trace.iter().all(|(_,at,_)| (0x21fffe..0x220001).contains(at)));
        }
    }
}

#[test]
fn size_counts_and_pointer_reads_cross_a_bank_without_wrapping() {
    let source=r#"MODULE SAMPLE
USE CSTRING.IMPL AS STR
SIZE length
PROC Main()
 length=STR.strlen(CSTRING(BYTE POINTER($21FFFE)))
RETURN
ENDMODULE"#;
    for optimize in [false,true] {
        let image=compile(source,optimize);
        let mut h=Harness::new(&image,&caller(image.entry),0);
        let mut bytes=vec![b'a';65538]; bytes.push(0);
        h.bus.map(0x21fffe,&bytes,false);
        assert!(h.cpu.run_until(&mut h.bus,100_000_000, |_| Inputs::default(),|cpu|cpu.is_stopped()).unwrap());
        h.guards(0);
        assert_eq!(h.global(&image,"length",3),65538);
    }
}

#[test]
fn compact_o65_literals_remain_valid_at_two_placements() {
    use actionc::mir65816::o65 as format;
    let source=r#"MODULE SAMPLE
USE CSTRING.IMPL AS STR
BYTE ARRAY mutable=c"abc"
CSTRING FUNC LiteralText() RETURN(c"a\x80\0tail")
LONGINT FUNC Main()
 CSTRING text
 text=LiteralText()
 IF text(1)#$80 THEN RETURN(10) FI
 IF text(3)#'t THEN RETURN(10) FI
 IF STR.strlen(text)#2 THEN RETURN(10) FI
 mutable(0)='z
 IF STR.strcmp(CSTRING(mutable),c"zbc")#0 THEN RETURN(10) FI
RETURN(LONGINT($12345678))
ENDMODULE"#;
    for optimize in [false,true] {
        let dir=Temp::new(); let path=dir.0.join("main.act"); std::fs::write(&path,source).unwrap();
        let p=actionc::compiler::native65816::prepare_file_with_entry(&path,optimize,&Default::default(),Some("Main")).unwrap();
        let bytes=p.compile_o65(&format::Options {profile:format::profile::COMPACT_ID.into(),..Default::default()}).unwrap().bytes;
        for variant in 0..2 {
            let placement=support::o65::placement(&bytes,variant,vec![support::o65::fault(variant)]);
            let image=format::compact::relocate(&bytes,&placement).unwrap();
            let mut h=Harness::new_compact(&image,&caller(image.entry),0,support::o65::fault(variant).address);
            h.run(); h.guards(0);
            assert_eq!(h.cpu.registers().a,0x5678);
            assert_eq!(h.cpu.registers().x,0x1234);
        }
    }
}
