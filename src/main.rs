use anyhow::{Context, Result};
use ramshield::{Config, Engine, dashboard};
use std::sync::Arc;
use tracing::info; // Add debug
use tracing_subscriber::EnvFilter;

/// CLI contract: `--config <path>` and a bare positional `<path>` both select
/// the config file. Unknown flags and missing values are FATAL — the old
/// parser silently dropped unrecognized arguments, so `./ramshield config.toml`
/// (positional) booted on compiled-in defaults with the file ignored.
fn parse_args(args: &[String]) -> Result<Option<String>> {
    let mut config_path: Option<String> = None;
    let mut i = 1; // skip program name
    while i < args.len() {
        match args[i].as_str() {
            "--config" | "-c" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("--config requires a path"))?;
                if config_path.is_some() {
                    anyhow::bail!("multiple config paths given");
                }
                config_path = Some(v.clone());
                i += 2;
            }
            s if s.starts_with('-') && s.len() > 1 => {
                anyhow::bail!("unknown argument: {s}");
            }
            _ => {
                if config_path.is_some() {
                    anyhow::bail!("multiple config paths given");
                }
                config_path = Some(args[i].clone());
                i += 1;
            }
        }
    }
    Ok(config_path)
}

#[tokio::main]
async fn main() -> Result<()> {
    ramshield::install_panic_hook();
    // Atomic P0: --version flag (BACKLOG #8) — checked before tracing init
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("ramshield {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("ramshield=info"));

    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    let config_path = parse_args(&args)?;

    let config = match config_path {
        Some(path) => {
            let absolute_path = std::fs::canonicalize(&path)
                .map_err(|e| anyhow::anyhow!("Error canonicalizing path {}: {}", path, e))?;
            eprintln!(
                "Attempting to load config from absolute path: {:?}",
                absolute_path
            );
            Config::load(
                absolute_path
                    .to_str()
                    .context("config path contains non-UTF-8 characters")?,
            )?
        }
        None => {
            // Still honor env overrides in no-config mode (dashboard auth etc).
            let mut c = Config::default();
            c.apply_env_overrides()?;
            // P1 fix (same class as Config::load): env overrides could set a
            // public bind with no secrets; the fail-closed guard must run on
            // the FINAL config here too, not just on the file path.
            c.validate()?;
            c
        }
    };
    // ponytail: Config::load() already calls apply_env_overrides() once
    // internally. A second call here was harmless (idempotent) but wasteful
    // and confusing — removed.
    // ponytail: Debug on Config leaks auth_keys. Print summary, not raw.
    info!(
        "Loaded config: ipc.auth_keys={}, dashboard.bind={}",
        config.ipc.auth_keys.len(),
        config.dashboard.http_addr
    );
    // P1-7: no TLS in the stack — surface any public-bind exposure at boot.
    for w in config.exposure_warnings() {
        tracing::warn!("{w}");
    }

    // Start RamShield normally
    let store = Arc::new(ramshield::storage::Store::new(config.engine.shard_count));
    store.traffic.ram_limit_mb.store(
        config.engine.ram_limit_mb,
        std::sync::atomic::Ordering::Relaxed,
    );
    // Store created_at for uptime tracking
    store
        .traffic
        .uptime_secs
        .store(1, std::sync::atomic::Ordering::Relaxed); // mark non-zero
    let metrics = Arc::new(ramshield::metrics::Metrics::with_block_log(
        config.dashboard.block_log_size,
    ));
    let engine = Arc::new(Engine::new(config.clone(), store.clone(), metrics.clone()));
    let _engine_handle = engine
        .clone()
        .start_async()
        .context("failed to start engine pipeline")?;

    // Periodic uptime updater (every second)
    {
        let started = std::time::Instant::now();
        let traffic = store.traffic.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                traffic.uptime_secs.store(
                    started.elapsed().as_secs(),
                    std::sync::atomic::Ordering::Relaxed,
                );
            }
        });
    }

    // Start dashboard if enabled — dedicated OS thread + tokio runtime
    // to guarantee responsiveness under detection load.
    let eng_clone = engine.clone();
    let dashboard_config = config.dashboard.clone();
    let config_clone = config.clone();
    if dashboard_config.enabled {
        let _dashboard_handle = std::thread::Builder::new()
            .name("rs-dashboard".into())
            .spawn(move || -> Result<()> {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_io()
                    .enable_time()
                    .build()
                    .context("failed to build dashboard runtime")?;
                rt.block_on(async move {
                    if let Err(e) =
                        dashboard::serve(eng_clone, &dashboard_config.http_addr, &config_clone)
                            .await
                    {
                        tracing::error!("Dashboard server error: {}", e);
                    }
                });
                Ok(())
            })
            .context("failed to spawn dashboard thread")?;
    }

    info!("RamShield running — Ctrl+C to stop");

    // Graceful shutdown trap: SIGINT (Ctrl+C), SIGTERM (systemd stop/kill
    // default), SIGHUP (terminal hangup / reload request). Any of the three
    // starts the same drain: engine.shutdown() → worker joins → enforcement
    // task exits → AyaXdpApplier dropped → Ebpf closed → kernel unbinds the
    // XDP program (RAII detach restores default stack forwarding).
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())?;
    tokio::select! {
        _ = sigint.recv() => info!("Received SIGINT; initiating graceful shutdown"),
        _ = sigterm.recv() => info!("Received SIGTERM; initiating graceful shutdown"),
        _ = sighup.recv() => info!("Received SIGHUP; initiating graceful shutdown"),
    }

    // Initiate graceful shutdown
    engine.shutdown();

    // F9: real joins (workers final-flush pre_aggs on exit) with a 5s grace,
    // instead of a fixed spin that neither guaranteed completion nor early-exit.
    // Blocking joins go through spawn_blocking — must not park the RT.
    let eng = engine.clone();
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(6),
        tokio::task::spawn_blocking(move || eng.join_workers(std::time::Duration::from_secs(5))),
    )
    .await;

    info!("Shutdown complete.");
    Ok(())
}

#[cfg(test)]
mod cli_tests {
    use super::parse_args;

    fn args(v: &[&str]) -> Vec<String> {
        std::iter::once("ramshield".to_string())
            .chain(v.iter().map(|s| s.to_string()))
            .collect()
    }

    #[test]
    fn config_flag_selects_path() {
        assert_eq!(
            parse_args(&args(&["--config", "a.toml"])).unwrap(),
            Some("a.toml".into())
        );
        assert_eq!(
            parse_args(&args(&["-c", "a.toml"])).unwrap(),
            Some("a.toml".into())
        );
    }

    #[test]
    fn positional_path_selects_config() {
        // Regression: the old parser silently dropped this — the file was
        // never loaded and the process booted on compiled-in defaults.
        assert_eq!(
            parse_args(&args(&["config.toml"])).unwrap(),
            Some("config.toml".into())
        );
    }

    #[test]
    fn unknown_flag_is_fatal() {
        // Fail-closed: a typo'd flag must not boot with an ignored argument.
        assert!(parse_args(&args(&["--confg", "a.toml"])).is_err());
        assert!(parse_args(&args(&["--x"])).is_err());
    }

    #[test]
    fn missing_flag_value_is_fatal() {
        assert!(parse_args(&args(&["--config"])).is_err());
    }

    #[test]
    fn duplicate_config_path_is_fatal() {
        assert!(parse_args(&args(&["--config", "a.toml", "b.toml"])).is_err());
        assert!(parse_args(&args(&["a.toml", "--config", "b.toml"])).is_err());
    }

    #[test]
    fn empty_is_none() {
        assert_eq!(parse_args(&args(&[])).unwrap(), None);
    }
}
