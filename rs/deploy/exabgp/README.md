# RamShield → ExaBGP

This is the upstream mitigation boundary for saturated links. RamShield does
not contain BGP credentials or a BGP state machine. It emits only validated
FlowSpec/RTBH commands to a root-owned FIFO; ExaBGP owns the BGP session.

Create the FIFO with restrictive ownership/permissions:

```sh
install -d -m 0750 /run/ramshield
mkfifo /run/ramshield/flowspec.fifo
chown ramshield:exabgp /run/ramshield/flowspec.fifo
chmod 0620 /run/ramshield/flowspec.fifo
```

Configure RamShield:

```toml
[upstream]
enabled = true
interface = "eth0"
link_capacity_mbps = 1000
saturation_pct = 90
bgp_mode = "flowspec"
bgp_fifo = "/run/ramshield/flowspec.fifo"
protected_prefix = "203.0.113.0/24"
```

Run `ramshield-api.py` as an ExaBGP API process and enable the appropriate
FlowSpec address family on the neighbor. ExaBGP's text API accepts commands of
the form `announce flow route { match { ... } then { discard; } }` and requires
a newline/flush after each command.

For RTBH, use:

```toml
bgp_mode = "rtbh"
bgp_community = "65000:666"
```

The BGP path is deliberately fail-closed at the command boundary: malformed
CIDRs/communities are rejected by RamShield configuration validation, the FIFO
write is non-blocking, and a missing ExaBGP reader never blocks the detection
engine.
