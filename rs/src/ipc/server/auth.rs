use super::*;

/// Parse `config.ipc.auth_keys` entries into (key_id, key_bytes).
/// Mirrors `Config::validate` so bind fails instead of silently shipping
/// with zero keys when an entry like `k1:badhex` is present.
pub(crate) fn parse_ipc_keys(
    config: &crate::config::Config,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = Vec::new();
    for entry in &config.ipc.auth_keys {
        let (id, hex_str) = entry
            .split_once(':')
            .ok_or_else(|| format!("ipc.auth_keys entry '{entry}' is not 'key_id:hex_key'"))?;
        if hex_str.is_empty() {
            return Err(format!("ipc.auth_keys[{id}] has empty hex key"));
        }
        if hex_str.len() % 2 != 0 {
            return Err(format!("ipc.auth_keys[{id}] has odd-length hex key"));
        }
        if hex_str.bytes().any(|b| !b.is_ascii_hexdigit()) {
            return Err(format!("ipc.auth_keys[{id}] contains non-hex characters"));
        }
        if hex_str.len() < 32 {
            return Err(format!(
                "ipc.auth_keys[{id}] hex key must be >= 32 chars (16 bytes)"
            ));
        }
        let bytes =
            hex::decode(hex_str).map_err(|e| format!("ipc.auth_keys[{id}] hex decode: {e}"))?;
        out.push((id.to_string(), bytes));
    }
    Ok(out)
}

/// Role hierarchy rank. Higher rank satisfies `require_at_least`.
fn role_rank(role: KeyRole) -> u8 {
    match role {
        KeyRole::Telemetry => 0,
        KeyRole::ReadOnly => 1,
        KeyRole::Operator => 2,
        KeyRole::Admin => 3,
    }
}

/// P2: minimum role required per request variant. Deny by default — any
/// variant added to `Request` later must be added here explicitly or it
/// requires Admin.
fn require_at_least(principal: &Principal, min: KeyRole) -> Result<(), String> {
    if role_rank(principal.role) >= role_rank(min) {
        Ok(())
    } else {
        Err("insufficient role".to_string())
    }
}

pub(crate) fn authorize(principal: &Principal, request: &Request) -> Result<(), String> {
    match request {
        Request::ReportConnection { .. } | Request::ReportConnections { .. } => {
            require_at_least(principal, KeyRole::Telemetry)
        }

        Request::CheckIp { .. }
        | Request::GetIpStats { .. }
        | Request::GetStats
        | Request::GetStatus => require_at_least(principal, KeyRole::ReadOnly),

        Request::BlockIp { .. }
        | Request::BlockCidr { .. }
        | Request::UnblockIp { .. }
        | Request::UnblockCidr { .. } => require_at_least(principal, KeyRole::Operator),

        Request::Flush => require_at_least(principal, KeyRole::Admin),
    }
}

/// Verify the HMAC auth envelope on a raw frame line.
/// Expected shape: `{"auth":{"key_id":..,"ts_ms":..,"sig":..},"type":..,...}`.
/// The signature covers `<ts_ms>.<full frame bytes minus the auth object>` —
/// simplest correct scheme: signer strips `auth` field, signs remaining JSON
/// bytes with ts prefix. Here we sign the RAW LINE as sent by the client
/// including its auth object? No — sig must cover payload WITHOUT auth object,
/// else self-reference. Client signs `ts.payload_without_auth`; server removes
/// the auth object, re-serializes compactly and compares.
///
/// P1: returns the authenticated `key_id` alongside the auth-stripped frame so
/// IPC enforcement commands can attribute their `actor` to the real principal
/// instead of a hard-coded `"admin"`.
pub(crate) fn verify_frame_auth(
    keys: &[(String, Vec<u8>)],
    line: &[u8],
    replay: &ramshield_protocol::auth::ReplayStore,
) -> Result<(serde_json::Value, String), &'static str> {
    let mut v: serde_json::Value =
        serde_json::from_slice(line).map_err(|_| "frame is not valid JSON")?;
    let auth = v
        .as_object_mut()
        .ok_or("frame is not an object")?
        .remove("auth")
        .ok_or("missing auth object")?;
    let obj = auth.as_object().ok_or("auth is not an object")?;
    let key_id = obj
        .get("key_id")
        .and_then(|x| x.as_str())
        .ok_or("auth.key_id missing")?;
    let ts_ms = obj
        .get("ts_ms")
        .and_then(|x| x.as_u64())
        .ok_or("auth.ts_ms missing")?;
    let sig = obj
        .get("sig")
        .and_then(|x| x.as_str())
        .ok_or("auth.sig missing")?;

    // Payload = compact serialization of the frame without the auth object.
    let payload = serde_json::to_vec(&v).map_err(|_| "reserialize failed")?;
    let principal =
        ramshield_protocol::auth::verify_authenticated(keys, key_id, ts_ms, sig, &payload, replay)?;
    Ok((v, principal.key_id))
}
