# RamShield — runtime-only image.
# The binary embeds its BPF ELF (crates/ramshield-xdp/build.rs → bpf-linker),
# so the image carries no build toolchain. Building the binary itself needs
# a nightly toolchain (BPF -Zbuild-std) — do it on the host and copy the
# result into the build context (scripts/build_docker.sh).
#
# Build context layout (assembled by the script):
#   ramshield            — prebuilt release binary
#   config.baseline.toml — trial config (loopback-only IPC/dashboard)
#
# Run:
#   docker run -d --name ramshield \
#     --cap-add NET_ADMIN --cap-add BPF --cap-add PERFMON \
#     -p 9999:9999 grep999/ramshield:0.2.0
# XDP attach needs the caps above (skb mode). Without them the daemon
# still serves IPC + dashboard; XDP reports inactive.
FROM debian:bookworm-slim

COPY ramshield /usr/local/bin/ramshield
COPY config.baseline.toml /etc/ramshield/config.toml

# IPC 7890 (loopback inside container), dashboard 9999
EXPOSE 9999

# bash exists in -slim; no curl needed for a TCP-level health probe
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s \
  CMD bash -c 'exec 3<>/dev/tcp/127.0.0.1/9999' || exit 1

ENTRYPOINT ["/usr/local/bin/ramshield"]
CMD ["--config", "/etc/ramshield/config.toml"]
