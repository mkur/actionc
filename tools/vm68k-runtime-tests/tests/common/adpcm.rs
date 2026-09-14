//! Native adapters record each checkpoint in ordinary, symbol-addressed arrays.
use super::{common, reference};
use actionc::compiler::native::{NativeCompileOptions, compile_file};
use actionc_vm68k_tests::Machine;
use reference::{bytes, replace_once, text, words};

struct Event {
    kind: u32,
    state: Vec<u32>,
}
struct Vector {
    label: String,
    command: u32,
    input: Vec<u32>,
    resets: Vec<u32>,
    events: Vec<Event>,
    report: Vec<u32>,
}
fn layout(vectors: &str) -> Vec<(&str, usize)> {
    vectors
        .lines()
        .find_map(|s| s.strip_prefix("# state "))
        .unwrap()
        .split_whitespace()
        .map(|s| {
            let (name, count) = s.split_once(':').unwrap();
            assert!(name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'));
            let count = count.parse().unwrap();
            assert!((1..=24).contains(&count));
            (name, count)
        })
        .collect()
}
fn vectors(source: &str, encode: bool) -> Vec<Vector> {
    let state_words: usize = layout(source).iter().map(|(_, n)| n).sum();
    assert_eq!(state_words, if encode { 92 } else { 83 });
    let mut result = Vec::new();
    let mut current: Option<Vector> = None;
    for line in source
        .lines()
        .filter(|s| !s.starts_with('#') && !s.is_empty())
    {
        let f: Vec<_> = line.split_whitespace().collect();
        match f[0] {
            "case" => {
                assert!(current.is_none());
                assert_eq!(f.len(), 5);
                current = Some(Vector {
                    label: f[1].into(),
                    command: f[2].parse().unwrap(),
                    input: words(&bytes(f[3]), if encode { 4 } else { 1 }),
                    resets: words(&bytes(f[4]), 1),
                    events: Vec::new(),
                    report: Vec::new(),
                });
            }
            "event" => {
                assert_eq!(f.len(), 3);
                let event = Event {
                    kind: f[1].parse().unwrap(),
                    state: words(&bytes(f[2]), 4),
                };
                assert_eq!(event.state.len(), state_words);
                current.as_mut().unwrap().events.push(event);
            }
            "end" => {
                assert_eq!(f.len(), 2);
                let mut v = current.take().unwrap();
                v.report = words(&bytes(f[1]), 4);
                assert_eq!(v.report.len(), if encode { 2 } else { 6 });
                assert_eq!(v.input.len(), v.resets.len() * if encode { 2 } else { 1 });
                assert!(v.resets.len() <= if encode { 128 } else { 256 });
                assert!(v.command <= if encode { 2 } else { 1 });
                let mut kinds = if encode && v.command == 0 {
                    vec![1, 3]
                } else {
                    vec![1]
                };
                for &reset in &v.resets {
                    assert!(reset <= 1);
                    if reset != 0 {
                        kinds.push(1);
                    }
                    kinds.push(2);
                }
                assert_eq!(v.events.iter().map(|e| e.kind).collect::<Vec<_>>(), kinds);
                result.push(v);
            }
            other => panic!("unknown vector record {other}"),
        }
    }
    assert!(current.is_none());
    assert_eq!(result.len(), 15);
    result
}

fn instrument(source: &str, vector_text: &str, encode: bool, capacity: usize) -> String {
    let mut source = text(source, false);
    let layout = layout(vector_text);
    let state_words: usize = layout.iter().map(|(_, n)| n).sum();
    let mut capture = format!(
        "INT result\n\
        BYTE testCommand\n\
        CARD testCount,testEvents\n\
        {} ARRAY testInput(256)\n\
        BYTE ARRAY testResets({}),testKinds({capacity})\n\
        LONGINT ARRAY testState({}),testReport({})\n\
        LONGINT testEncoded\n\
        PROC Capture(BYTE kind)\n\
          BYTE i\n\
          CARD base\n\
          base=testEvents*{state_words}\n",
        if encode { "LONGINT" } else { "BYTE" },
        if encode { 128 } else { 256 },
        capacity * state_words,
        if encode { 2 } else { 6 }
    );
    let mut offset = 0;
    for (name, size) in layout {
        let name = if name == "encoded" {
            "testEncoded"
        } else {
            name
        };
        if size == 1 {
            capture.push_str(&format!("  testState(base+{offset})={name}\n"));
        } else {
            capture.push_str(&format!(
                "  FOR i=0 TO {} DO testState(base+{offset}+i)={name}(i) OD\n",
                size - 1
            ));
        }
        offset += size;
    }
    capture.push_str("  testKinds(testEvents)=kind\n  testEvents==+1\nRETURN\n");
    replace_once(&mut source, "INT result\n", &capture);
    if encode {
        replace_once(
            &mut source,
            "    compressed(i)=Encode(test_data(2*i),test_data(2*i+1))\n",
            "    compressed(i)=Encode(test_data(2*i),test_data(2*i+1))\n    testEncoded=compressed(i)\n    Capture(2)\n",
        );
        replace_once(
            &mut source,
            "  FOR i=0 TO 22 DO tqmf(i)=0 OD\nRETURN",
            "  FOR i=0 TO 22 DO tqmf(i)=0 OD\n  Capture(1)\nRETURN",
        );
        replace_once(
            &mut source,
            "  FOR i=0 TO 2 DO test_data(i)=10*Cos32(LONGINT(2000)*3141*i) OD\nRETURN",
            "  FOR i=0 TO 2 DO test_data(i)=10*Cos32(LONGINT(2000)*3141*i) OD\n  Capture(3)\nRETURN",
        );
    } else {
        replace_once(
            &mut source,
            "  accumd(0)=xs\nRETURN",
            "  accumd(0)=xs\n  Capture(2)\nRETURN",
        );
        replace_once(
            &mut source,
            "  FOR i=0 TO 10 DO accumc(i)=0 accumd(i)=0 OD\nRETURN",
            "  FOR i=0 TO 10 DO accumc(i)=0 accumd(i)=0 OD\n  Capture(1)\nRETURN",
        );
    }
    replace_once(&mut source, "PROC Main()\n", "PROC Benchmark()\n");
    let compute = if encode {
        "IF testCommand=1 THEN\n\
           testEncoded=Encode(testInput(2*i),testInput(2*i+1))\n\
         ELSE testEncoded=Quantl(testInput(2*i),testInput(2*i+1)) FI\n\
         Capture(2)"
    } else {
        "Decode(testInput(i))"
    };
    replace_once(
        &mut source,
        "ENDMODULE\n",
        &format!(
            "\
        PROC Main()\n\
          CARD i\n\
          IF testCommand={} THEN\n\
            Benchmark()\n\
            testReport(0)=result testReport(1)=checksum\n\
            {}\n\
          ELSE\n\
            Reset()\n\
            IF testCount>0 THEN\n\
              FOR i=0 TO testCount-1 DO\n\
                IF testResets(i)#0 THEN Reset() FI\n\
                {compute}\n\
              OD\n\
            FI\n\
          FI\n\
        RETURN\nENDMODULE\n",
            if encode { 0 } else { 1 },
            if encode {
                ""
            } else {
                "FOR i=0 TO 3 DO testReport(2+i)=samples(i) OD"
            }
        ),
    );
    source
}

pub fn execute(source: &str, vector_text: &str, encode: bool, optimize: bool) {
    let vector_text = text(vector_text, optimize);
    let vectors = vectors(&vector_text, encode);
    let capacity = vectors.iter().map(|v| v.events.len()).max().unwrap();
    let state_words: usize = layout(&vector_text).iter().map(|(_, n)| n).sum();
    assert!(capacity * state_words < 65536); // CARD snapshot offsets cannot wrap.
    let source = common::Source::new(&text(
        &instrument(&text(source, optimize), &vector_text, encode, capacity),
        optimize,
    ));
    let image = compile_file(
        &source.0,
        &NativeCompileOptions {
            optimize,
            project_root: Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")),
            ..Default::default()
        },
    )
    .unwrap()
    .image;
    let module = if encode { "ADPCM_ENC" } else { "ADPCM_DEC" };
    let symbol = |name: &str| image.symbol(&format!("{module}.{name}")).unwrap();
    let mut checkpoints = 0;
    for v in vectors {
        let label = format!("{module}/{optimize}/{}", v.label);
        let mut vm = Machine::from_image(&image).unwrap();
        vm.write_scalar(symbol("testCommand"), v.command).unwrap();
        vm.write_scalar(symbol("testCount"), v.resets.len() as u32)
            .unwrap();
        let mut input = vec![if encode { 0xcccccccc } else { 0xcc }; 256];
        input[..v.input.len()].copy_from_slice(&v.input);
        let mut resets = vec![0xcc; if encode { 128 } else { 256 }];
        resets[..v.resets.len()].copy_from_slice(&v.resets);
        let mut state = vec![0xcccccccc; capacity * state_words];
        let mut kinds = vec![0xcc; capacity];
        for (name, values) in [
            ("testInput", &input),
            ("testResets", &resets),
            ("testState", &state),
            ("testKinds", &kinds),
        ] {
            vm.write_array(symbol(name), values).unwrap();
        }
        vm.write_array(symbol("testReport"), &vec![0xcccccccc; v.report.len()])
            .unwrap();
        let run = vm.run(100_000_000);
        assert!(
            matches!(run.outcome, actionc_vm68k_tests::Outcome::Completed),
            "{label}: {run:#?}"
        );
        for (i, event) in v.events.iter().enumerate() {
            state[i * state_words..(i + 1) * state_words].copy_from_slice(&event.state);
            kinds[i] = event.kind;
        }
        for (name, expected) in [
            ("testInput", &input),
            ("testResets", &resets),
            ("testState", &state),
            ("testKinds", &kinds),
            ("testReport", &v.report),
        ] {
            let actual = vm.read_array(symbol(name)).unwrap();
            assert_eq!(actual.len(), expected.len());
            for (i, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                assert_eq!(actual, expected, "{label}/{name}/{i}");
            }
        }
        for (name, expected) in [
            ("testCommand", v.command),
            ("testCount", v.resets.len() as u32),
            ("testEvents", v.events.len() as u32),
        ] {
            assert_eq!(
                vm.read_scalar(symbol(name)).unwrap(),
                expected,
                "{label}/{name}"
            );
        }
        checkpoints += v.events.len();
    }
    eprintln!("{module}/{optimize}: {checkpoints} complete state checkpoints");
    assert_eq!(checkpoints, if encode { 1561 } else { 2713 });
}
