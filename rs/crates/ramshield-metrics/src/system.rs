use super::*;

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn with_system<F, R>(f: F) -> R
where
    F: FnOnce(&mut System) -> R,
{
    static SYS: Mutex<Option<System>> = Mutex::new(None);
    // ponytail: poison-recovery — sysinfo cache is advisory; a panicked
    // holder must not take down every dashboard poll. Upgrade: parking_lot.
    let mut guard = SYS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if guard.is_none() {
        *guard = Some(System::new_all());
    }
    match guard.as_mut() {
        Some(sys) => f(sys),
        None => unreachable!("SYS initialized above"),
    }
}

/// (cpu_usage, total_ram_mb, own_process_rss_mb). Cached 1s — see get_system_usage.
pub fn get_system_usage() -> (f32, usize, usize) {
    // ponytail: 1s TTL cache — dashboard polls snapshot+modules per cycle and
    // both need the same numbers; upgrade to crossbeam channel ticker if
    // sub-second freshness ever matters.
    static CACHE: Mutex<Option<(std::time::Instant, f32, usize, usize)>> = Mutex::new(None);
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, cpu, mem, rss)) =
        (*cache).filter(|(at, ..)| at.elapsed() < std::time::Duration::from_secs(1))
    {
        return (cpu, mem, rss);
    }
    let fresh = with_system(|sys| {
        // CPU% needs two samples spaced ~200ms+; refresh_specifics avoids the
        // full process-table walk of refresh_all() on every dashboard poll.
        sys.refresh_specifics(
            sysinfo::RefreshKind::nothing()
                .with_cpu(sysinfo::CpuRefreshKind::nothing().with_cpu_usage())
                .with_memory(sysinfo::MemoryRefreshKind::everything()),
        );
        let cpu_usage = sys.global_cpu_usage();
        // sysinfo 0.30+: total_memory() returns bytes (was KB before).
        let total_memory_mb = (sys.total_memory() / (1024 * 1024)) as usize;
        let rss_mb = sys
            .process(
                sysinfo::get_current_pid()
                    .ok()
                    .unwrap_or(sysinfo::Pid::from(0)),
            )
            .map(|p| p.memory() / (1024 * 1024))
            .unwrap_or(0) as usize;
        (cpu_usage, total_memory_mb, rss_mb)
    });
    *cache = Some((std::time::Instant::now(), fresh.0, fresh.1, fresh.2));
    fresh
}
