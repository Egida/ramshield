//! The daemon's pipeline description must match what the daemon actually
//! builds.
//!
//! `src/README.md` listed "Forecasting — `Forecaster::new(...)` spawns a
//! tokio task" as a boot-pipeline step. The daemon never constructs a
//! `Forecaster`; only the forecasting crate's own tests do. A reader
//! believed forecasting was an active mitigation path.
//!
//! The check reads both sides: the README must not claim the forecaster
//! runs, and the pipeline must not construct one. If a future change wires
//! forecasting in for real, this test fails and the README gets updated
//! with it.

const README: &str = include_str!("../src/README.md");
const BOOT: &str = include_str!("../src/engine/boot.rs");

#[test]
fn readme_does_not_claim_forecasting_runs_in_the_pipeline() {
    assert!(
        !README.contains("**Forecasting** —"),
        "src/README.md still lists Forecasting as a boot-pipeline step"
    );
    assert!(
        README.contains("Forecasting is not in the pipeline"),
        "src/README.md must say that the daemon does not start the forecaster"
    );
}

#[test]
fn the_daemon_does_not_construct_a_forecaster() {
    assert!(
        !BOOT.contains("Forecaster"),
        "the boot pipeline now constructs a Forecaster - update src/README.md and this test together"
    );
}
