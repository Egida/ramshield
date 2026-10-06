# RamShield 1.0 Release Contract

RamShield 1.0 is a **single-node Linux ingress defense product**. The supported
contract is narrower than the repository workspace: experimental multi-node,
analytics, forecasting, and relative-detection features are not automatically
part of the supported mitigation promise.

## Required before 1.0

1. Claims in README, feature matrices, and threat-class documentation are a
   subset of behavior covered by tests and qualification evidence.
2. Pinned CI is green for formatting, build/check, Clippy, workspace tests,
   dependency audit, and dependency policy checks.
3. Production installation does not expose administrative IPC/dashboard access
   without authentication.
4. Requested XDP mode is honored; native/driver mode is never silently
   downgraded.
5. Configured active XDP whose projection is stale is visible as degraded or
   failed according to fallback policy and never reports fully healthy.
6. SIGKILL/restart recovery restores IP and CIDR enforcement state from
   checkpoint plus WAL tail, including unblock transitions.
7. Upgrade qualification passes on a real Linux host. XDP qualification is
   performed on the actual kernel/NIC/driver combinations that will be used.
8. Release identity is consistent across Cargo version, tag, image, installer
   artifact names, and signed release metadata.

## Relative detection

Relative detection is **experimental and opt-in**. It is not a default 1.0
detection guarantee. The implementation is bounded by finite configuration
validation, representable sample maturity, prior-baseline evaluation, and a
consecutive-breach requirement. Production use requires workload soak evidence.

## Scope

RamShield is not a cloud scrubbing service, global DDoS network, or substitute
for upstream capacity protection. The product protects a Linux host while that
host still has visibility and enforcement capacity.
