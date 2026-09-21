use super::*;
use actionc::mir65816::o65::{self as format, profile::*};

pub fn compile(source: &str, optimize: bool, imports: Vec<Binding>) -> Vec<u8> {
    prepare(source, optimize)
        .compile_o65(&Options {
            imports,
            ..Default::default()
        })
        .unwrap()
        .bytes
}
pub fn placement(
    bytes: &[u8],
    variant: usize,
    providers: Vec<format::Provider>,
) -> format::Placement {
    let file = format::decode(bytes).unwrap();
    let mut bases = if variant == 0 {
        [0x100000, 0x32fffc, 0x41fffc]
    } else {
        [0x600000, 0x8410f8, 0x91fffc]
    };
    for i in 0..3 {
        if file.lengths[i] == 0 {
            bases[i] = 0;
        }
    }
    format::Placement {
        bases,
        allowed: vec![format::Region {
            address: 0x10000,
            size: 0xff0000,
        }],
        reserved: vec![format::Region {
            address: 0x040000,
            size: 0x1000,
        }],
        nmi_extra_stack: 0,
        providers,
    }
}
pub fn fault(variant: usize) -> format::Provider {
    format::Provider {
        name: OVERFLOW.into(),
        address: if variant == 0 { 0x048000 } else { 0x068000 },
        size: 2,
        contract: Contract::overflow(),
    }
}
pub fn object(image: &format::RelocatedImage, name: &str) -> u32 {
    let o = image
        .profile()
        .objects
        .iter()
        .find(|o| {
            o.name.eq_ignore_ascii_case(name)
                || o.name
                    .to_ascii_uppercase()
                    .contains(&format!("_{}_", name.to_ascii_uppercase()))
        })
        .unwrap_or_else(|| panic!("missing {name}: {:?}", image.profile().objects));
    image.location(o.location)
}
pub fn routine(image: &format::RelocatedImage, name: &str) -> u32 {
    let r = image
        .profile()
        .routines
        .iter()
        .find(|o| {
            o.name.eq_ignore_ascii_case(name)
                || o.name
                    .to_ascii_uppercase()
                    .contains(&format!("_{}_", name.to_ascii_uppercase()))
        })
        .unwrap();
    image.routine_address(r)
}
pub fn record(
    name: &str,
    optimize: bool,
    bytes: &[u8],
    placement: &format::Placement,
    image: &format::RelocatedImage,
    cycles: u64,
    stack: Option<u32>,
) {
    if let Ok(directory) = std::env::var("A816_QUALIFICATION_DIR") {
        let stem =
            Path::new(&directory).join(format!("o65-{name}-{optimize}-{:06x}", placement.bases[0]));
        std::fs::write(stem.with_extension("o65"), bytes).unwrap();
        std::fs::write(
            stem.with_extension("placement.json"),
            serde_json::to_vec_pretty(placement).unwrap(),
        )
        .unwrap();
        let file = format::decode(bytes).unwrap();
        let descriptor = file
            .exports
            .iter()
            .find(|e| e.name == DESCRIPTOR)
            .unwrap()
            .value;
        let facts = serde_json::json!({"file_bytes":bytes.len(),"text":file.lengths[0],"data":file.lengths[1],"bss":file.lengths[2],"descriptor":file.lengths[0]-descriptor,"relocations":file.relocations.len(),"code_bytes":image.profile().routines.iter().map(|r|r.size).sum::<u32>(),"cycles":cycles,"observed_stack":stack,"entry":image.entry(),"fault":image.stack_overflow()});
        std::fs::write(
            stem.with_extension("metrics.json"),
            serde_json::to_vec_pretty(&facts).unwrap(),
        )
        .unwrap();
    }
}
