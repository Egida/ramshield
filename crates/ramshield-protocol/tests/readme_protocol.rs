//! I-024: the README must document the MAC input the code actually uses.
//!
//! The README documented `HMAC-SHA256(key, "{ts_ms}:{payload}")` while the
//! implementation signs `ts_ms` ++ "." ++ key_id ++ payload, and it quoted a
//! 30-second skew window where the code allows 10. An implementer following
//! the README produced signatures the server rejects, and one following its
//! skew figure shipped a replay window twice as wide as the code's.
//!
//! This test reads the README and checks the facts it states against the
//! code's own constants, so the documentation cannot drift back.

use ramshield_protocol::auth::MAX_CLOCK_SKEW_MS;

const README: &str = include_str!("../README.md");

#[test]
fn readme_states_the_real_mac_input_layout() {
    // The README spells the layout out in prose and in the formula below it.
    assert!(
        README.contains("`ts_ms` ++ `\".\"` ++ `key_id` ++ `payload`"),
        "the README must state the MAC input as ts_ms ++ \".\" ++ key_id ++ payload"
    );
    assert!(
        README.contains("HMAC-SHA256(key, \"<ts_ms>.<key_id><payload>\")"),
        "the README must give the signing formula"
    );
    assert!(
        !README.contains("\"{ts_ms}:{payload}\""),
        "the README still documents the old, wrong signing string \"{{ts_ms}}:{{payload}}\""
    );
}

#[test]
fn readme_states_the_real_skew_window() {
    let secs = MAX_CLOCK_SKEW_MS / 1000;
    assert!(
        README.contains(&format!("({secs} seconds)")),
        "the README must quote the real MAX_CLOCK_SKEW_MS of {secs} seconds"
    );
    assert!(
        !README.contains("(30 seconds)"),
        "the README still quotes a 30-second skew window; the code allows {secs}"
    );
}
