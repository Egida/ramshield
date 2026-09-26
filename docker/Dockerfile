# RamShield — runtime image.
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
#       grep999/ramshield:0.3.0
# XDP attach needs the caps above (skb mode). Without them the daemon
# still serves IPC + dashboard; XDP reports inactive.
FROM debian:bookworm-slim
ARG XDP_ENABLED=true
ENV XDP_ENABLED=$XDP_ENABLED

RUN apt-get update \
 && apt-get install -y --no-install-recommends libcap2-bin \
 && rm -rf /var/lib/apt/lists/*

# Dedicated non-root service account, no shell (mirrors deploy/systemd).
RUN groupadd -r ramshield && useradd -r -g ramshield -d /var/lib/ramshield -s /usr/sbin/nologin ramshield

COPY ramshield /usr/local/bin/ramshield
COPY config.baseline.toml /etc/ramshield/config.toml

# If XDP disabled, turn off xdp.enabled in the config file.
RUN if [ "$XDP_ENABLED" = "false" ]; then \
    sed -i '/^\[xdp\]/,$ s/^enabled = true$/enabled = false/' /etc/ramshield/config.toml; \
  fi

# File capabilities let the non-root daemon attach XDP (skb mode) when the
# container is started with --cap-add NET_ADMIN --cap-add BPF --cap-add PERFMON.
RUN setcap 'cap_net_admin,cap_perfmon,cap_bpf+eip' /usr/local/bin/ramshield

# IPC 7890 stays loopback inside the container; dashboard 9999 is exposed.
EXPOSE 9999

USER ramshield

ENTRYPOINT ["/usr/local/bin/ramshield"]
CMD ["--config", "/etc/ramshield/config.toml"]

LABEL org.opencontainers.image.title="RamShield" \
      org.opencontainers.image.description="Autonomous ingress defense daemon" \
      org.opencontainers.image.version="0.3.0" \
      org.opencontainers.image.source="https://github.com/grep999/ramshield" \
      org.opencontainers.image.licenses="MIT"