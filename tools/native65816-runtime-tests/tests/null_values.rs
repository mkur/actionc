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

#[test]
fn pointer_array_elements_keep_all_lanes_and_null_stores_do_not_copy_records() {
    let source = r#"
TYPE Row=[CARD index BYTE ARRAY payload(30)]
TYPE Collection=[BYTE tag Row POINTER ARRAY refs(2) BYTE tail]
Collection holder=$7600
Row selected
Row POINTER input=$7100
Row POINTER ARRAY slots=$7300
BYTE POINTER ARRAY bytes=$7400
PROC POINTER ARRAY handlers=$7500
ADDRESS ARRAY results=$7200
BYTE callbackEmpty=$7220

PROC Clear(Row POINTER ARRAY values CARD index)
  values(index)=NULL
RETURN

PROC Main()
  selected.index=1
  slots(0)=input slots(1)=input slots(2)=input
  results(0)=ADDRESS(slots(selected.index))
  slots(selected.index)=NULL
  results(1)=ADDRESS(slots(1)) results(2)=ADDRESS(slots(2))
  Clear(slots,2)
  results(3)=ADDRESS(slots(2))
  bytes(0)=BYTE POINTER(input) bytes(1)=NULL
  results(4)=ADDRESS(bytes(0)) results(5)=ADDRESS(bytes(1))
  holder.tag=$31 holder.tail=$73
  holder.refs(0)=input holder.refs(1)=NULL
  results(6)=ADDRESS(holder.refs(0)) results(7)=ADDRESS(holder.refs(1))
  handlers(0)=NULL callbackEmpty=handlers(0)=NULL
RETURN
"#;
    for optimize in [false, true] {
        for guarded in [false, true] {
            let mut options = layout();
            options.stack_checks = guarded;
            let image = prepare(source, optimize).compile(&options).unwrap().image;
            let crlf = prepare(&source.replace('\n', "\r\n"), optimize)
                .compile(&options).unwrap().image;
            assert_eq!(image.to_json().unwrap(), crlf.to_json().unwrap());
            let caller = caller(image.entry);
            for value in [0u32, 1, 0x100, 0xffff, 0x10000, 0xabcdef, 0xffffff] {
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller, mask);
                    h.bus.map(0x7400, &[0;0x220], true);
                    h.bus.ram[0x7100..0x7103].copy_from_slice(&value.to_le_bytes()[..3]);
                    for (start,end) in [(0x71ff,0x7219),(0x72ff,0x730a),(0x73ff,0x7407),
                                        (0x74ff,0x7507),(0x75ff,0x7609)] {
                        h.bus.ram[start..end].fill(0xa5);
                    }
                    h.bus.watched.insert(value);
                    h.run();
                    h.guards(mask);
                    for (index,expected) in [value,0,value,0,value,0,value,0].into_iter().enumerate() {
                        assert_eq!(h.bus.value(0x7200+index as u32*3,3), expected);
                    }
                    assert_eq!(h.bus.value(0x7300,3),value);
                    assert_eq!(&h.bus.ram[0x7303..0x7309], &[0;6]);
                    assert_eq!(h.bus.value(0x7400,3),value);
                    assert_eq!(&h.bus.ram[0x7403..0x7406], &[0;3]);
                    assert_eq!(&h.bus.ram[0x7500..0x7506], &[0,0,0,0xa5,0xa5,0xa5]);
                    assert_eq!(h.bus.value(0x7601,3),value);
                    assert_eq!(&h.bus.ram[0x7604..0x7607], &[0;3]);
                    assert_eq!((h.bus.ram[0x7600],h.bus.ram[0x7607]),(0x31,0x73));
                    assert_eq!(h.bus.ram[0x7220],1);
                    for address in [0x71ff,0x7218,0x72ff,0x7309,0x73ff,0x7406,
                                    0x74ff,0x7506,0x75ff,0x7608] {
                        assert_eq!(h.bus.ram[address],0xa5,"canary {address:x}");
                    }
                    assert!(h.bus.trace.is_empty(),"pointer element accessed its pointee");
                }
            }
        }
    }
}
