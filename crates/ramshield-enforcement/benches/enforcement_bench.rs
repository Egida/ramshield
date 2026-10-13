//! Minimal enforcement microbenchmarks for A-037 regression budgets.
//! This intentionally uses only the standard library so the bench adds no
//! dependency to the core-only build. Run with: cargo bench -p ramshield-enforcement --features xdp.

use std::hint::black_box;
use std::time::{Duration, Instant};

fn measure(name: &str, iterations: u64, mut operation: impl FnMut()) {
    // Warm up the operation before recording the measured interval.
    for _ in 0..1_000 {
        operation();
    }

    let started = Instant::now();
    for _ in 0..iterations {
        operation();
    }
    let elapsed = started.elapsed();
    report(name, iterations, elapsed);
}

fn report(name: &str, iterations: u64, elapsed: Duration) {
    let nanos_per_op = elapsed.as_nanos() as f64 / iterations as f64;
    println!("{name}: {iterations} iterations in {elapsed:?} ({nanos_per_op:.1} ns/op)");
}

fn main() {
    let ip = std::net::IpAddr::V4(std::net::Ipv4Addr::new(10, 0, 0, 1));

    measure("blocklist_key_create", 100_000, || {
        black_box(ramshield_enforcement::xdp::BlocklistKey::from_ip(\n            black_box(ip),\n        ));
    });

    measure("ip_parsing", 100_000, || {
        black_box(black_box("10.0.0.1").parse::<std::net::Ipv4Addr>().unwrap());
    });

    measure("wal_entry_construct", 100_000, || {
        black_box(ramshield_storage::wal::WalEntry::BlockIp {
            ip: "10.0.0.1".into(),
            reason: "benchmark".into(),
            ttl_secs: Some(100),
            ts_ns: 42,
        });
    });
}
