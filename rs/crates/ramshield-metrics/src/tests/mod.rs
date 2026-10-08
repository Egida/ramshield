use super::*;

#[test]
fn xdp_projection_stale_gauge_is_exported() {
    let m = Metrics::new();
    m.set_xdp_projection_active(true);
    assert!(
        m.render_prometheus()
            .contains("ramshield_xdp_projection_stale 1")
    );
    m.set_xdp_projection_active(false);
    assert!(
        m.render_prometheus()
            .contains("ramshield_xdp_projection_stale 0")
    );
}

#[test]
fn enforcement_drops_counter_is_exported() {
    let m = Metrics::new();
    m.inc_enforcement_dropped();
    m.inc_enforcement_dropped();
    assert_eq!(m.enforcement_dropped.load(Ordering::Relaxed), 2);
    let out = m.render_prometheus();
    assert!(
        out.contains("ramshield_enforcement_dropped_total 2"),
        "enforcement drops must be scrapeable — a silently dropped security \
             command is otherwise invisible to the operator"
    );
}

#[test]
fn xdp_apply_failures_counter_is_exported() {
    // The mirror of enforcement_dropped for the dataplane leg: when the
    // CIDR LPM trie fills (no LRU, hard cap) a subnet block silently stops
    // reaching the wire. This counter is the only scrapeable signal that
    // happened.
    let m = Metrics::new();
    m.inc_xdp_apply_failures();
    m.inc_xdp_apply_failures();
    m.inc_xdp_apply_failures();
    assert_eq!(m.xdp_apply_failures.load(Ordering::Relaxed), 3);
    let out = m.render_prometheus();
    assert!(
        out.contains("ramshield_xdp_apply_failures_total 3"),
        "XDP apply failures must be scrapeable — a silently full CIDR trie \
             otherwise looks like the subnet mitigation is simply not firing"
    );
}

#[test]
fn block_log_evicts_at_configured_cap() {
    let m = Metrics::with_block_log(5);
    for i in 0..12 {
        m.record_block(&format!("10.0.0.{i}"), "high_rps", "detection");
    }
    let log = m.block_log.lock().unwrap();
    assert_eq!(log.len(), 5, "ring must evict oldest beyond cap");
    // newest survives, oldest gone
    assert_eq!(log.back().unwrap().ip, "10.0.0.11");
    assert_eq!(log.front().unwrap().ip, "10.0.0.7");
}

#[test]
fn block_log_cap_floor_is_one() {
    // zero/nonsense config must not produce a zero-capacity deadlock ring
    let m = Metrics::with_block_log(0);
    m.record_block("10.0.0.1", "high_rps", "detection");
    assert_eq!(m.block_log.lock().unwrap().len(), 1);
}
