//! The forecasting README must not claim an API the crate does not have.
//!
//! The README named a `Forecaster::tick()` method that does not exist and
//! a `Store::get_batch_history()` source that is not on `Store`. Both
//! claims were in the crate's own documentation and both were wrong.
//!
//! The guard is deliberately narrow: it checks the claims that were
//! wrong, not the prose.

use ramshield_forecasting::Hypothesis;

fn readme() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/README.md");
    std::fs::read_to_string(path).expect("the forecasting README must be readable")
}

#[test]
fn the_readme_names_no_method_that_does_not_exist() {
    let text = readme();
    assert!(
        !text.contains("Forecaster::tick()"),
        "the crate has no `tick()` method; the loop is `run()`"
    );
    assert!(
        !text.contains("get_batch_history"),
        "`get_batch_history` is not on `Store`; the forecaster reads the live traffic counters"
    );
}

#[test]
fn the_readme_lists_the_real_hypothesis_variants() {
    let text = readme();
    for variant in [
        Hypothesis::Normal,
        Hypothesis::VolumetricDoS,
        Hypothesis::SlowRampDoS,
        Hypothesis::FlashCrowd,
    ] {
        let name = format!("{variant:?}");
        assert!(
            text.contains(&name),
            "the README must name the `{name}` variant; the enum has it"
        );
    }
    // the old names are not variants and must not come back
    for stale in ["Volumetric,", "SlowRamp,"] {
        assert!(
            !text.contains(stale),
            "`{stale}` is not a variant of `Hypothesis`"
        );
    }
}

#[test]
fn the_readme_says_the_daemon_does_not_run_the_crate() {
    let text = readme();
    assert!(
        text.contains("does not run this crate"),
        "the README must say the daemon does not construct a `Forecaster`"
    );
}
