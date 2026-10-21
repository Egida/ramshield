# ramshield-xdp

## Why it exists

This crate is a build artifact container. It compiles the BPF program (Rust, in `ramshield-xdp-bpf/src/main.rs`, built with `aya-ebpf`) into an ELF binary at build time, then exports it as a static byte slice. The actual loading, attaching to a network interface, and map management happens in `ramshield-enforcement::xdp::AyaXdpApplier`.

This separation exists because BPF compilation needs `bpf-linker` and the `bpfel-unknown-none` target at build time — tools that aren't always available on the target machine. `build.rs` looks for `bpf-linker` on `PATH` and then in `~/.local/bin`; if it finds neither, it warns and skips the BPF build rather than failing the workspace. The `xdp` feature flag controls whether this crate is included.

## What it does

```rust
pub static BPF_ELF: &[u8] = aya::include_bytes_aligned!(
    concat!(env!("OUT_DIR"), "/ramshield-xdp")
);
```

That's the entire public API. One static byte slice containing the compiled BPF ELF. `build.rs` runs a nested `cargo build --target=bpfel-unknown-none` on the BPF crate through `bpf-linker` and places the result in `OUT_DIR`. The `aya` crate's `include_bytes_aligned!` macro loads it at compile time with correct alignment for BPF map access.

## How it's consumed

`ramshield-enforcement` is the sole consumer:

```rust
// In ramshield-enforcement::xdp
pub struct AyaXdpApplier { ... }

impl AyaXdpApplier {
    pub fn load_and_attach(&mut self) -> Result<(), EnforcementError> {
        let mut bpf = aya::Bpf::load(ramshield_xdp::BPF_ELF)?;
        // ... attach to interface, get blocklist map
    }
}
```

The enforcement crate defines the BPF map key type (`BlocklistKey`) and writes the map values. There is no `BlocklistValue` type: the map value is a plain `u64` — the absolute expiry timestamp on the monotonic clock, with `u64::MAX` meaning permanent. This keeps the type definitions next to the code that writes to them — one source of truth, no drift between the BPF program and the userspace writer.

## BPF Program Behavior

The compiled kernel program runs on every incoming packet at the XDP hook point (before the kernel's network stack). It:

1. Extracts the source IP from the packet header.
2. Looks the source IP up in the flat blocklist map. The key is the raw
   wire bytes of the address copied into a 16-byte key — there is no
   hash, so the userspace writer and the kernel program agree on the
   key by construction.
3. If the map holds the address **and its expiry is still in the
   future**, drops the packet (returns `XDP_DROP`). An entry whose
   expiry has passed does not drop the packet, so an expired block stops
   blocking on its own.
4. Otherwise checks the CIDR maps (`BLOCKCIDR` / `BLOCKCIDR6`, LPM
   tries holding /24 and /64 entries). A match there drops the packet
   even when the address is not in the flat map.
5. If neither matched, passes it through (returns `XDP_PASS`).

This happens in kernel space — no context switch to userspace, no memory copies. For blocked IPs, the packet is dropped before any kernel processing (no socket allocation, no TCP state machine, no buffer allocation).

## Uniqueness

**Build artifact, not a runtime component.** This crate does nothing at runtime. It exists solely to isolate the BPF compilation step — a build-time concern — from the rest of the workspace. The 9 lines of Rust are a delivery mechanism for a binary artifact.

**Fail-open design.** If the BPF program fails to load (wrong kernel, missing permissions), `AyaXdpApplier::load_and_attach()` returns an error. The enforcement service catches this and falls back to in-band enforcement (Store-based blocking). The system degrades gracefully — slower but functional.

## Dependencies

**Reads from:** `aya` (BPF loading, `include_bytes_aligned!`).

**Written by:** nothing at runtime.

**Read by:** `ramshield-enforcement::xdp::AyaXdpApplier` (sole consumer).

## Benchmarks

No benchmarks — this crate has no runtime code. Kernel-side packet processing benchmarks would require a live XDP environment and are planned for a future phase.

## Testing

No unit tests — the crate's correctness is verified by `ramshield-enforcement` integration tests that load the BPF program and verify block/unblock behavior through the `AyaXdpApplier`.
