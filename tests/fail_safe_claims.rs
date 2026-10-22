//! `FAIL_SAFE_BEHAVIOR.md` must state the postures the code actually has.
//!
//! The document used to claim a global fail-closed posture - "any
//! uncertainty or failure defaults to blocking traffic, never allowing it
//! through" - while the BPF program returns `XDP_PASS` on the paths it
//! cannot parse. These checks pin the document to the source: the
//! dataplane's fail-open paths, the authentication path's rejections, the
//! store's refusal under the RAM budget, and the capture path's degraded
//! state.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} must be readable: {e}", path.display()))
}

#[test]
fn the_document_does_not_claim_a_global_fail_closed_posture() {
    let doc = read("FAIL_SAFE_BEHAVIOR.md");
    assert!(
        !doc.contains("never allowing it through"),
        "the document must not claim that nothing is ever allowed through: \
         the XDP dataplane fails open on packets it cannot parse"
    );
    assert!(
        doc.contains("no single fail-closed posture"),
        "the document must say up front that the subsystems differ"
    );
    assert!(
        doc.contains("fails open on packets it cannot judge"),
        "the document must state the dataplane's fail-open behaviour"
    );
}

#[test]
fn the_dataplane_really_fails_open_on_the_paths_the_document_names() {
    let bpf = read("crates/ramshield-xdp/ramshield-xdp-bpf/src/main.rs");
    let pass = bpf.matches("XDP_PASS").count();
    let drop = bpf.matches("XDP_DROP").count();
    assert!(
        pass >= 4,
        "the BPF program must keep its XDP_PASS paths, the document names them: {pass}"
    );
    assert!(
        drop >= 2,
        "the BPF program must still drop classified packets"
    );
}

#[test]
fn the_authentication_path_really_fails_closed() {
    let auth = read("crates/ramshield-protocol/src/auth.rs");
    for rejection in [
        "timestamp outside allowed skew window",
        "signature mismatch",
        "signature length mismatch",
    ] {
        assert!(
            auth.contains(rejection),
            "the auth path must reject {rejection}: the document says it fails closed"
        );
    }
    assert!(
        auth.contains("MAX_CLOCK_SKEW_MS"),
        "the skew window the document names must exist"
    );
}

#[test]
fn the_store_really_refuses_rather_than_drops() {
    let write = read("crates/ramshield-storage/src/store/write.rs");
    assert!(
        write.contains("CapacityExceeded"),
        "the store must refuse a net-new key under the RAM budget: the document says it refuses rather than drops"
    );
}

#[test]
fn the_capture_path_really_degrades_instead_of_dying() {
    let engine = read("src/engine/mod.rs");
    assert!(
        engine.contains("native capture stopped"),
        "a dead capture must say so in the health reason: the document says it degrades and keeps serving"
    );
    let metrics = read("crates/ramshield-metrics/src/prometheus.rs");
    assert!(
        metrics.contains("ramshield_native_capture_workers_running"),
        "the capture worker count must be exported: the document says the capture state is visible"
    );
}
