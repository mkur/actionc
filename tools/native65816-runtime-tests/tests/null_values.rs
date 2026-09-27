mod support;
use support::*;

const SOURCE: &str = r#"
TYPE Node=[BYTE tag BYTE POINTER next]
Node holder
BYTE POINTER input=$7100,output=$7110
BYTE ARRAY results=$7200
BYTE calls
CSTRING text
PROC POINTER callback
BYTE POINTER initial=[NULL]

BYTE POINTER FUNC Fetch()
  calls==+1
RETURN(input)

BYTE POINTER FUNC Empty()
RETURN(NULL)

Node POINTER FUNC EmptyNode()
RETURN(NULL)

PROC Take(BYTE POINTER value)
  results(4)=value=NULL
RETURN

PROC Main()
  calls=0
  results(0)=input=NULL
  results(1)=NULL<>input
  results(2)=Fetch()=NULL
  results(3)=NULL=Fetch()
  Take(NULL)
  output=NULL
  holder.tag=$7f
  holder.next=NULL
  results(5)=Empty()=NULL
  results(6)=EmptyNode()=NULL
  text=NULL
  results(7)=text=NULL
  callback=NULL
  results(8)=callback=NULL
  LET BYTE POINTER saved=NULL
  results(9)=saved=NULL
  results(10)=initial=NULL
  results(11)=holder.next=NULL
  results(12)=holder.tag
  results(13)=calls
RETURN
"#;

#[test]
fn null_values_execute_with_full_pointer_width_and_exact_stores() {
    for optimize in [false, true] {
        let image = compile(SOURCE, optimize);
        assert_eq!(image.to_json().unwrap(),
            compile(&SOURCE.replace('\n', "\r\n"), optimize).to_json().unwrap());
        let caller = caller(image.entry);
        // Include pointers with only their bank byte set: they are not NULL.
        for value in [0u32, 1, 0x100, 0xffff, 0x10000, 0xffffff] {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller, mask);
                h.bus.ram[0x7100..0x7103].copy_from_slice(&value.to_le_bytes()[..3]);
                h.bus.ram[0x710f..0x7114].fill(0xa5);
                h.bus.ram[0x7200..0x7210].fill(0xa5);
                h.run();
                h.guards(mask);
                let zero = u8::from(value == 0);
                assert_eq!(&h.bus.ram[0x7200..0x720e],
                    &[zero, 1-zero, zero, zero, 1, 1, 1, 1, 1, 1, 1, 1, 0x7f, 2],
                    "opt={optimize}, pointer={value:x}, mask={mask}");
                assert_eq!(&h.bus.ram[0x710f..0x7114], &[0xa5, 0, 0, 0, 0xa5]);
                assert_eq!(&h.bus.ram[0x720e..0x7210], &[0xa5, 0xa5]);
            }
        }
    }
}
