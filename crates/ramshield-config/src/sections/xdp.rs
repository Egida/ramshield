use crate::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdpConfig {
    /// Attach the XDP kernel program. When false, enforcement is in-band only.
    #[serde(default)]
    pub enabled: bool,
    /// Interface to attach to (e.g. "eth0", "lo").
    #[serde(default = "default_xdp_iface")]
    pub interface: String,
    /// "skb" (generic, works everywhere) or "drv" (native, production NICs).
    #[serde(default = "default_xdp_mode")]
    pub mode: String,
    /// When XDP attach fails, continue with in-band enforcement (DEGRADED).
    /// Default false: configured XDP that is not attached is FAILED /healthz 503.
    #[serde(default)]
    pub allow_inband_fallback: bool,
}

impl Default for XdpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interface: default_xdp_iface(),
            mode: default_xdp_mode(),
            allow_inband_fallback: false,
        }
    }
}

fn default_xdp_iface() -> String {
    "eth0".into()
}

fn default_xdp_mode() -> String {
    "skb".into()
}
