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

/// The server re-serializes the frame after removing `auth`, and
/// `serde_json` is built without `preserve_order`, so that
/// re-serialization is key-sorted. The client never signs the bytes it
/// sent; it signs the sorted form. A README that says "sign exactly the
/// bytes of the frame minus its auth object" sends an implementer down a
/// path the server rejects.
#[test]
fn readme_states_the_key_sorted_signing_rule() {
    assert!(
        README.contains("key-sorted serialization of the frame"),
        "the README must state that the signed payload is the key-sorted serialization"
    );
    assert!(
        README.contains("The server does not MAC the bytes you sent"),
        "the README must state that the server re-serializes rather than using the client's bytes"
    );
    assert!(
        README.contains("sort_keys=True"),
        "the README must show the key-sorted form a client can actually produce"
    );
    assert!(
        !README.contains("must sign exactly the bytes of the frame"),
        "the README still says the client signs its own bytes, which the server never MACs"
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
