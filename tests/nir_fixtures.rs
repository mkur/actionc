use std::collections::BTreeSet;
use std::path::Path;

mod nir_fixture_support;
mod snapshot_support;

use nir_fixture_support::{
    NIR_FIXTURE_CASES, REQUIRED_EXECUTABLE_FEATURES, collect_features, format_feature_inventory,
    lower_case, snapshot_path, structural_coverage_programs,
};

fn fixture_features(repo_root: &Path) -> BTreeSet<nir_fixture_support::NirFeature> {
    let mut features = BTreeSet::new();
    for case in NIR_FIXTURE_CASES {
        features.extend(collect_features(&lower_case(repo_root, *case)));
    }
    for program in structural_coverage_programs(repo_root) {
        features.extend(collect_features(&program));
    }
    features
}

#[test]
fn nir_fixtures_match_snapshots() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(!NIR_FIXTURE_CASES.is_empty(), "expected NIR fixtures");

    for case in NIR_FIXTURE_CASES {
        let program = lower_case(repo_root, *case);
        let actual = actionc::nir::format_program(&program);
        let expected_path = snapshot_path(repo_root, *case);
        let expected = snapshot_support::read_snapshot(&expected_path);

        assert_eq!(
            actual,
            expected,
            "NIR fixture `{}` changed for {}\n\nupdate {} deliberately",
            case.name,
            case.source,
            expected_path.display()
        );
    }
}

#[test]
fn nir_fixture_feature_inventory_matches_snapshot() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let features = fixture_features(repo_root);
    let actual = format_feature_inventory(&features);
    let expected_path = repo_root.join("fixtures/nir/coverage.txt");
    let expected = snapshot_support::read_snapshot(&expected_path);
    assert_eq!(
        actual,
        expected,
        "NIR fixture feature inventory changed; update {} deliberately",
        expected_path.display()
    );
}

#[test]
fn executable_nir_feature_floor_is_covered() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let features = fixture_features(repo_root);
    let missing = REQUIRED_EXECUTABLE_FEATURES
        .iter()
        .filter(|feature| !features.contains(feature))
        .collect::<Vec<_>>();

    assert!(
        missing.is_empty(),
        "positive NIR fixtures lost required executable shapes: {missing:?}"
    );
}
