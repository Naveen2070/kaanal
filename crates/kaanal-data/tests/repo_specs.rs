//! Invalid output is a bug (FR-DATA-05): every `OpenAPI` spec bundled with the repo is sampled
//! with many seeds and must validate against its own schema. Body-budget cuts are not misses.
//! The local petstore acceptance spec is included when present (`internal/` is git-ignored).

use std::path::Path;

use kaanal_data::selfcheck::check_endpoint;
use kaanal_data::{ApiModel, Options, Pack};

const SEEDS: usize = 256;
/// Seeds per endpoint for the packs after the first (the default locale gets [`SEEDS`]).
const PACK_SEEDS: usize = 48;
/// Locale packs the specs are also sampled with (names and places differ, constraints must not).
const PACKS: &[&str] = &["en-IN", "de-DE", "pt-BR"];

const SPECS: &[&str] = &[
    "crates/kaanal/demo/openapi.yaml",
    "crates/kaanal/tests/fixtures/openapi.yaml",
    "crates/kaanal-server/tests/fixtures/openapi.yaml",
    "examples/gatewayless/openapi.yaml",
    "examples/nginx-server/openapi.yaml",
    "internal/acceptance/petstore/openapi.json",
];

#[test]
fn bundled_specs_generate_valid_responses() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut checked = 0;
    let mut bad = Vec::new();
    for rel in SPECS {
        let Ok(src) = std::fs::read_to_string(root.join(rel)) else {
            assert!(rel.starts_with("internal/"), "{rel} is missing");
            continue;
        };
        let model = ApiModel::from_openapi(&src).unwrap_or_else(|e| panic!("{rel}: {e}"));
        for e in &model.endpoints {
            checked += 1;
            let base = u64::try_from(checked).unwrap();
            for (i, tag) in PACKS.iter().enumerate() {
                let opts = Options {
                    pack: Pack::builtin(tag).unwrap(),
                    ..Options::default()
                };
                let seeds = if i == 0 { SEEDS } else { PACK_SEEDS };
                for f in check_endpoint(e, &opts, seeds, base) {
                    if !f.budget {
                        bad.push(format!(
                            "{rel} [{tag}]: {} {} {}",
                            f.endpoint, f.path, f.keyword
                        ));
                    }
                }
            }
        }
    }
    assert!(checked > 0);
    bad.sort();
    bad.dedup();
    assert!(
        bad.is_empty(),
        "{} distinct misses:\n{}",
        bad.len(),
        bad.join("\n")
    );
}
