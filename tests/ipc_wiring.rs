use ramshield::{Config, Engine};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

#[test]
fn ipc_wiring_binds_and_accepts() {
    let mut cfg = Config::default();
    cfg.ipc.tcp_addr = "127.0.0.1:17890".into(); // off default port — don't collide

    let store = Arc::new(ramshield::storage::Store::new(cfg.engine.shard_count));
    let metrics = Arc::new(ramshield::metrics::Metrics::new());
    let engine = Arc::new(Engine::new(cfg.clone(), store, metrics));
    let _handle = engine.clone().start_async().expect("start");

    // Give the OS thread time to bind
    std::thread::sleep(Duration::from_secs(1));

    let mut stream = TcpStream::connect("127.0.0.1:17890").expect("TCP connect to IPC server");

    writeln!(stream, r#"{{"type":"get_status"}}"#).expect("write");
    let mut resp = String::new();
    BufReader::new(&stream).read_line(&mut resp).expect("read");

    // Should get valid JSON response
    let v: serde_json::Value = serde_json::from_str(&resp).expect("valid JSON");
    assert_eq!(v["type"], "ok");
    assert_eq!(v["message"], "ok");

    engine.shutdown();
}

#[test]
fn ipc_wiring_rejects_invalid_json() {
    let mut cfg = Config::default();
    cfg.ipc.tcp_addr = "127.0.0.1:17891".into();

    let store = Arc::new(ramshield::storage::Store::new(cfg.engine.shard_count));
    let metrics = Arc::new(ramshield::metrics::Metrics::new());
    let engine = Arc::new(Engine::new(cfg.clone(), store, metrics));
    let _handle = engine.clone().start_async().expect("start");
    std::thread::sleep(Duration::from_secs(1));

    let mut stream = TcpStream::connect("127.0.0.1:17891").expect("connect");
    writeln!(stream, "not json").expect("write");
    let mut resp = String::new();
    BufReader::new(&stream).read_line(&mut resp).expect("read");

    let v: serde_json::Value = serde_json::from_str(&resp).expect("valid JSON");
    assert_eq!(v["type"], "error");
    assert_eq!(v["code"], 1);

    engine.shutdown();
}

#[test]
fn ipc_wiring_oversize_connection_gets_error_frame_not_bare_reset() {
    // F4: a client pushing past max_connection_bytes must receive a typed
    // 413 error frame before the server closes, instead of a bare RST.
    let mut cfg = Config::default();
    cfg.ipc.tcp_addr = "127.0.0.1:17892".into();

    let store = Arc::new(ramshield::storage::Store::new(cfg.engine.shard_count));
    let metrics = Arc::new(ramshield::metrics::Metrics::new());
    let engine = Arc::new(Engine::new(cfg.clone(), store, metrics));
    let _handle = engine.clone().start_async().expect("start");
    std::thread::sleep(Duration::from_secs(1));

    let mut stream = TcpStream::connect("127.0.0.1:17892").expect("connect");
    // Push >1MB (default max_connection_bytes) with no newline — cap trips first.
    let chunk = vec![b'a'; 65536];
    for _ in 0..20 {
        if stream.write_all(&chunk).is_err() {
            break; // server may close mid-send after sending the error frame
        }
        let _ = stream.flush();
    }

    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut resp = String::new();
    // Error frame should already be in flight / buffered before close.
    match BufReader::new(&stream).read_line(&mut resp) {
        Ok(0) => panic!("server closed without sending error frame"),
        Ok(_) => {
            let v: serde_json::Value =
                serde_json::from_str(&resp).expect("error frame is valid JSON");
            assert_eq!(v["type"], "error");
            assert_eq!(v["code"], 413);
            assert!(
                v["message"]
                    .as_str()
                    .unwrap()
                    .contains("max_connection_bytes")
            );
        }
        Err(e) => panic!("read failed before any frame arrived: {e}"),
    }

    engine.shutdown();
}

/// An unterminated line must not be allowed to buffer past the configured
/// line limit when that limit is the tighter of the two bounds.
///
/// By default `max_line_length` equals `max_connection_bytes`, and the
/// connection byte budget is then the binding constraint (see
/// `ipc_wiring_oversize_connection_gets_error_frame_not_bare_reset`). When an
/// operator configures a line limit BELOW the byte budget, the line limit
/// becomes the binding constraint, and an unterminated line must be stopped
/// at the line limit rather than being allowed to grow to the byte budget.
///
/// The check must look at the untermined TAIL only: the buffer can hold
/// several complete pipelined frames, so its total length is not the length
/// of any one line.
#[test]
fn ipc_wiring_unterminated_line_is_capped_at_the_tighter_line_limit() {
    let mut cfg = Config::default();
    cfg.ipc.tcp_addr = "127.0.0.1:17893".into();
    // The line limit is TIGHTER than the byte budget here.
    cfg.ipc.max_line_length = Some(4096);
    cfg.ipc.max_connection_bytes = Some(1024 * 1024);

    let store = Arc::new(ramshield::storage::Store::new(cfg.engine.shard_count));
    let metrics = Arc::new(ramshield::metrics::Metrics::new());
    let engine = Arc::new(Engine::new(cfg.clone(), store, metrics));
    let _handle = engine.clone().start_async().expect("start");
    std::thread::sleep(Duration::from_secs(1));

    let mut stream = TcpStream::connect("127.0.0.1:17893").expect("connect");
    // A long unterminated line: no newline is ever sent.
    let chunk = vec![b'a'; 8192];
    for _ in 0..4 {
        if stream.write_all(&chunk).is_err() {
            break;
        }
        let _ = stream.flush();
    }

    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("timeout");
    let mut resp = String::new();
    match BufReader::new(&stream).read_line(&mut resp) {
        Ok(0) => panic!("server closed without sending an error frame"),
        Ok(_) => {
            let v: serde_json::Value =
                serde_json::from_str(&resp).expect("error frame is valid JSON");
            assert_eq!(v["type"], "error");
            assert_eq!(v["code"], 413);
            assert!(
                v["message"].as_str().unwrap().contains("max_line_length"),
                "expected the line-limit error, got: {resp}"
            );
        }
        Err(e) => panic!("read failed before any frame arrived: {e}"),
    }

    engine.shutdown();
}
