use std::path::PathBuf;
use volna_trace::OpenSpec;
use volna_trace::data::ScopeSizes;
use volna_trace::testing::brute_scope_size;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
fn counts_equal_brute_force_on_the_fixtures() {
    let mut aliased = 0;
    for rel in [
        "counter.vtr",
        "picorv32.vtr",
        "feature_showcase.vtr",
        "pipeline_showcase.vtr",
        "landing.vtr",
        "landing_dram.vtr",
        "values.vtr",
    ] {
        let session = OpenSpec::Path(root().join(rel)).open().unwrap();
        let h = session.hierarchy();
        let sizes = ScopeSizes::count(h);
        for s in 0..h.scope_count() {
            let want = brute_scope_size(h, s);
            assert_eq!(sizes.get(s), Some(want), "{rel}: {}", h.scope(s).name);
            aliased += usize::from(want.variables > want.signals);
        }
    }
    assert!(aliased > 0, "some fixture scope names a signal twice");
}
