//! Public compiler transport remains version 2 with flattened array metadata.
mod common;
#[path = "common/compiler.rs"]
mod compiler;
use actionc::{compiler::native::artifacts::NativeArtifact, mir68k::image::ImageView};
use actionc_vm68k_tests::Machine;

#[test]
fn multidimensional_public_artifacts_execute_with_flat_metadata_after_source_removal() {
    for (crlf, options) in [(false, &["--no-opt"][..]), (true, &[][..])] {
        let input = "BYTE ARRAY volume(2,3,5) CARD ARRAY grid(2,3)=[1 2 3 4]\nCARD result\nPROC Main()\nvolume(1,2,4)=77 grid(1,2)=42 result=grid(1,2)\nRETURN\n";
        let source = common::Source::new(&input.replace('\n', if crlf { "\r\n" } else { "\n" }));
        let path = source.0.parent().unwrap().join("bundle/image.json");
        compiler::compile(&source.0, &path, options);
        let manifest = std::fs::read_to_string(&path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&manifest).unwrap();
        assert_eq!(json["version"], 2);
        assert!(!manifest.contains("dimensions"));
        std::fs::remove_file(&source.0).unwrap();
        let artifact = NativeArtifact::load(&path).unwrap();
        artifact.verify().unwrap();
        for (name, count, stride) in [("grid", 6, 2), ("volume", 30, 1)] {
            let array = artifact.symbol(name).unwrap().array.as_ref().unwrap();
            assert_eq!((array.count, array.stride), (Some(count), stride));
            assert!(array.descriptor);
        }
        let mut vm = Machine::from_image(&artifact).unwrap();
        assert_eq!(
            vm.read_array(artifact.symbol("grid").unwrap()).unwrap(),
            [1, 2, 3, 4, 0, 0]
        );
        vm.run(100_000).assert_completed();
        assert_eq!(
            vm.read_array(artifact.symbol("grid").unwrap()).unwrap(),
            [1, 2, 3, 4, 0, 42]
        );
        assert_eq!(
            vm.read_array(artifact.symbol("volume").unwrap()).unwrap()[29],
            77
        );
        assert_eq!(
            vm.read_scalar(artifact.symbol("result").unwrap()).unwrap(),
            42
        );
    }
}
