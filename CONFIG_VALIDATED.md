# Production Config Validation

## Validation Status: ✅ COMPLETE

### Verified Configurations

#### 1. Production Config Template (`config.prod.toml.example`)
- **Status**: ✅ Validated
- **Path**: `/home/m/vehicle_of_rationalism/ramshield/beta/rs/config.prod.toml.example`
- **Contents**: Production-ready settings template
  - Binds to `0.0.0.0` for external exposure
  - WAL enabled at `/var/lib/ramshield/wal`
  - Detection threshold raised to 5000 RPS
  - RAM limit set to 8 GiB
  - XDP disabled (host-NIC deployment)

#### 2. Development Config (`config.toml`)
- **Status**: ✅ Validated
- **Path**: `/home/m/vehicle_of_rationalism/ramshield/beta/rs/config.toml`
- **Contents**: Local development settings
  - Binds to `127.0.0.1:7890` (loopback)
  - WAL disabled for local dev
  - Detection threshold at 500 RPS

#### 3. XDP Config (`config-xdp.toml`)
- **Status**: ✅ Validated
- **Path**: `/home/m/vehicle_of_rationalism/ramshield/beta/rs/config-xdp.toml`
- **Contents**: XDP-optimized settings
  - XDP enabled with `skb` mode
  - Binds to `127.0.0.1:7890`
  - Suitable for kernel testing

### Config Validation Checklist

#### Security Boundaries ✅ COMPLETED
- [x] Public bind requires authentication (`admin_password_hash`, `auth_keys`)
- [x] Negative auth tests implemented
- [x] Config validation enforces auth before external exposure

#### Capability Matrix ✅ COMPLETED
- [x] `cap_net_admin` - Required for XDP attach
- [x] `cap_perfmon` - Required for kernel monitoring
- [x] `cap_bpf` - Required for BPF map creation
- [x] `+eip` - Extended instruction set for eBPF

#### Runtime Verification ✅ COMPLETED
- [x] Binary capabilities restored after build (`setcap 'cap_net_admin,cap_perfmon,cap_bpf+eip' target/release/ramshield`)
- [x] Capabilities applied: `cap_net_admin,cap_perfmon,cap_bpf=eip`
- [x] Capability restoration script in `scripts/build_logged.sh`
- [x] Pre-push gate validation in `scripts/pre_push_gate.sh`

### Production Config Migration Path

#### Step 1: Environment Setup
```bash
# Create production config from template
cp config.prod.toml.example config.prod.toml

# Set appropriate permissions
chmod 600 config.prod.toml

# Initialize WAL directory
mkdir -p /var/lib/ramshield/wal
chown ramshield:ramshield /var/lib/ramshield/wal
```

#### Step 2: Configuration Hardening
```bash
# Add HMAC auth keys
echo "k1:<64-hex-hmac-key>" >> config.prod.toml

# Add admin password hash
echo "admin_password_hash = \"$(argon2 CLI hash)\"" >> config.prod.toml
```

#### Step 3: Runtime Capability Application
```bash
# Ensure capabilities persist across builds
sudo setcap 'cap_net_admin,cap_perfmon,cap_bpf+eip' target/release/ramshield
```

### Production Compliance Matrix

| Control | Status | Evidence |
|---------|--------|----------|
| File Capabilities | ✅ COMPLIANT | Binary has cap_net_admin,cap_perfmon,cap_bpf=eip |
| Config Validation | ✅ COMPLIANT | config.prod.toml.example template validated |
| Documentation | ⚠️ PARTIAL | Core docs exist, some sections need updates |
| Capability Restoration | ✅ COMPLIANT | Automated in build scripts |

### Next Steps for Full Compliance

1. **Deploy Config.prod.toml** as the active production configuration
2. **Update systemd service** to use production config path
3. **Add monitoring** for capability loss events
4. **Document rollback procedure** for capability restoration
5. **Create config validation CI/CD pipeline**

### Verification Commands

```bash
# Validate capabilities
getcap target/release/ramshield

# Check config syntax
python3 -c "import toml; toml.load('config.prod.toml')"

# Validate production template structure
python3 <<'PY'
import toml
config = toml.load('config.prod.toml.example')
required_sections = ['engine', 'ipc', 'dashboard', 'wal', 'forecasting', 'detection', 'xdp']
assert all(section in config for section in required_sections)
print("✅ Production config structure validated")
PY
```