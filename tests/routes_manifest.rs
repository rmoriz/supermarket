//! Asserts the committed `harness/routes.tsv` is exactly the router's route table.

use std::collections::BTreeSet;

#[test]
fn committed_manifest_matches_the_router() {
    let manifest = std::fs::read_to_string("harness/routes.tsv").expect("harness/routes.tsv");
    let committed: BTreeSet<String> = manifest.lines().map(str::to_owned).collect();
    let actual: BTreeSet<String> = supermarket_rs::routes::table::ROUTES
        .iter()
        .map(|r| format!("{}\t{}\t{}", r.method, r.path, r.action))
        .collect();

    let missing: Vec<_> = actual.difference(&committed).collect();
    let extra: Vec<_> = committed.difference(&actual).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "manifest drifted; missing: {missing:?}; extra: {extra:?}\nregenerate with: cargo run --bin routes_manifest > harness/routes.tsv"
    );
}

#[test]
fn manifest_covers_the_api() {
    let manifest = std::fs::read_to_string("harness/routes.tsv").unwrap();
    let api = manifest.lines().filter(|l| l.contains("\t/api/")).count();
    assert_eq!(api, 27, "api/v1 route count changed");
}
