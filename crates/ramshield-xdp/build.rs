//! Compile the XDP BPF object via aya-ebpf + bpf-linker.
//!
//! Produces a BPF ELF at OUT_DIR/ramshield-xdp for include_bytes_aligned!.
//! All loading, attaching, and map management lives in
//! ramshield-enforcement::xdp::AyaXdpApplier.
//!
//! XDP requested → real ELF. No placeholder. Build failure is a cargo failure.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=ramshield-xdp-bpf/src/main.rs");
    println!("cargo:rerun-if-changed=ramshield-xdp-bpf/Cargo.toml");
    println!("cargo:rerun-if-changed=bpf/main.rs");

    let out_dir = match env::var("OUT_DIR") {
        Ok(v) => PathBuf::from(v),
        Err(_) => panic!("OUT_DIR unset"),
    };
    let dest = out_dir.join("ramshield-xdp");

    if !try_aya_build(&dest) {
        panic!(
            "XDP BPF build failed: bpf-linker missing or cargo bpf build failed.\n\
             Install bpf-linker: https://github.com/aya-rs/bpf-linker/releases\n\
             Then: PATH=\"$HOME/.local/bin:$PATH\" cargo build --features full\n\
             No placeholder ELF is written."
        );
    }
    validate_elf(&dest);
}

fn find_bpf_linker() -> bool {
    if Command::new("bpf-linker")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return true;
    }
    if let Some(home) = env::var_os("HOME") {
        let p = PathBuf::from(home).join(".local/bin/bpf-linker");
        if p.is_file() {
            return Command::new(&p)
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
        }
    }
    false
}

fn try_aya_build(dest: &Path) -> bool {
    if !find_bpf_linker() {
        eprintln!("cargo:warning=bpf-linker not found");
        return false;
    }
    let manifest = Path::new("ramshield-xdp-bpf/Cargo.toml");
    if !manifest.exists() {
        eprintln!("cargo:warning=ramshield-xdp-bpf/Cargo.toml missing");
        return false;
    }
    let mut cmd = Command::new("cargo");
    if let Some(home) = env::var_os("HOME") {
        let extra = PathBuf::from(home).join(".local/bin");
        let path = env::var("PATH").unwrap_or_default();
        cmd.env(
            "PATH",
            format!("{}:{path}", extra.display()),
        );
    }
    let status = cmd
        .current_dir("ramshield-xdp-bpf")
        .args([
            "build",
            "--release",
            "--target=bpfel-unknown-none",
            "-Z",
            "build-std=core",
        ])
        .status();
    let Ok(s) = status else {
        return false;
    };
    if !s.success() {
        return false;
    }
    let candidates = [
        PathBuf::from("ramshield-xdp-bpf/target/bpfel-unknown-none/release/ramshield-xdp-bpf"),
        PathBuf::from("ramshield-xdp-bpf/target/bpfel-unknown-none/release/ramshield-xdp"),
    ];
    for c in candidates {
        if c.exists() {
            return std::fs::copy(&c, dest).is_ok();
        }
    }
    false
}

fn validate_elf(path: &Path) {
    let data = std::fs::read(path).unwrap_or_default();
    if data.len() < 64 || !data.starts_with(&[0x7f, b'E', b'L', b'F']) {
        panic!(
            "BPF ELF at {} is missing or not a valid ELF ({} bytes)",
            path.display(),
            data.len()
        );
    }
}
