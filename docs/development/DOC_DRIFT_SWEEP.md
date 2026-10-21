# I-025 — documentation drift sweep across the crate READMEs

**Status:** OPEN, partially worked. **62 claims: 30 fixed, 32 open.** The
open ones are spread over six crate READMEs: `ramshield-config` 1,
`ramshield-storage` 8, `ramshield-metrics` 7, `ramshield-forecasting` 8,
`ramshield-analytics` 6, `ramshield-xdp` 2. This is the result of a read-only
sweep of the claims in backlog row I-025 ("Dependency/documentation
drift"), carried out on branch `fix/i-013-synproxy-fixture-parity` at
`bf01372` (origin/master was `bd417cd` at the time of the sweep).

**Fixed since this file was written:** the `ramshield-config` README, all
eleven of its claims (F9 to F19), and four of the five `ramshield-xdp`
claims (F26, F27, F28, F30). The config template is also pinned by a test:
`crates/ramshield-config/tests/readme_template.rs` loads the ```toml
block straight out of the README through `Config::from_toml_file`, which
parses and validates it. The old template failed that test - the phantom
`[l7.rule]` section is a TOML parse error.

**An earlier version of this header was wrong, and said so.** It credited
the whole `ramshield-types`, `ramshield-forecasting`, `ramshield-xdp` and
`ramshield-config` READMEs as fixed, and counted 32 fixed / 21 open. A
re-verification of every claim against `origin/master` found that only
**17** were actually corrected: one of the seven types claims (F6) and one
of the nine forecasting claims (F31). The other fifteen of those claims
are still open, and they are listed below like any other. The lesson is
the one this file keeps needing: a fixed README is not a fixed claim, and
a count written by hand is a claim about the code that rots the moment
the code moves.

**Method.** Every claim was checked against the source by `git show`,
`grep -n` / `grep -rn` for callers, and `cat -n` for exact lines. No
build or test was run: the test counts below are static `#[test]` marker
counts, not run results.

**Scope note.** 54 defects were found across 8 crate READMEs. Two were
fixed in the same pass that produced this file, because they are
operator-facing and wrong in the direction that matters:

- **`crates/ramshield-types/README.md`** listed four `BlockReason`
  variants and two wire tokens (`rate_limit`, `pulse_wave`) that no code
  emits. The real enum has eight variants; the README now lists them and
  their tokens. This is the report's **F6** in the body.
- **`crates/ramshield-forecasting/README.md`** had the hypothesis index
  map inverted: it said H1 was FlashCrowd and H3 Volumetric. The code is
  the other way round, and it blocks on H1 (volumetric) and H2 (slow ramp)
  while deliberately NOT blocking on H3 (flash crowd). An operator
  reading the README concluded the exact opposite of what the daemon does.
  This is the report's **"F1 (CRITICAL)"** in the *Worst three* list.

**A warning about the report's own numbering.** The report reuses some
`F` numbers for two different findings: `F1`, `F6` and `F28` each appear
twice with different meanings (once in the body, once in the *Worst
three* list). Cite the file and the claim, not the number.

**Fixed since this file was written.** The sweep's other
operator-facing claims have been corrected on this branch, each against
the source:

- **`crates/ramshield-forecasting/README.md`** — the `Hypothesis`
  variant names (`VolumetricDoS`, `SlowRampDoS`; there is no
  `Volumetric` or `SlowRamp` variant), the loop method (`run()`, not
  `tick()`), the data source (`store.traffic`, not
  `Store::get_batch_history()`), and a note that the daemon does not
  construct a `Forecaster` at all. See the `Status: FIXED` line under
  F31 and the *Worst three* "F1".
- **`crates/ramshield-xdp/README.md`** — the build prerequisites
  (`bpf-linker`, not `clang`/`llvm-strip`), the phantom
  `BlocklistValue` type, and the drop path. See F26, F27, F28 and F30.
- **`crates/ramshield-types/README.md`** — the `BlockReason` variants
  and their wire tokens. See F6.

Everything else is recorded below, unfixed. They are documentation
defects, not code defects: no production behaviour depends on them, and
the fixes are per-file, per-claim edits.

**How to work this list.** Each entry gives the file and line, the claim
as written, and what the code actually says. Fix them in file-sized
batches, one crate at a time, and keep a guard test per crate only if the
crate's claims are load-bearing (a wire vocabulary, a public API shape).
A README that lists a struct's fields is worth a guard; a README that
counts its own tests is not.

---

# I-025 claim audit — RamShield

Audited tree: `/home/m/.cache/rs-master`, branch `fix/i-013-synproxy-fixture-parity`, HEAD `bf01372` (origin/master was `bd417cd` at the time). Read-only; no builds/tests run. Test counts below are static `#[test]` marker counts, not run results.

Method: `git show`/file reads on the audited tree, `grep -n`/`grep -rn` for callers, `cat -n` for exact lines. Every finding cites the command output I actually got.

---

## Sub-claim 1 — types error dependency split

**Cleared (partially):** there is no separate error crate. `crates/ramshield-types/Cargo.toml:7-16` = `thiserror`, `serde`, `uuid`, plus empty feature `storage = []` (no consumer enables it — dependents grep shows none passing `features=["storage"]`).

**F1 (MEDIUM)** `crates/ramshield-types/README.md:168`
> "**Dependencies:** `uuid`."

Code: `Cargo.toml:8-10` → `thiserror = "1"`, `serde = {…derive}`, `uuid = {…v4,serde}`. Two deps undocumented.
**Status: FIXED.** The README now names `serde`, `thiserror` and `uuid`, the dev-dependency `serde_json`, and the `storage` feature that no dependent enables. `crates/ramshield-types/tests/readme_claims.rs` checks the list against `Cargo.toml`.


**F2 (MEDIUM)** `crates/ramshield-types/README.md:172`
> "**Read by:** every other crate in the workspace."

Code: dependents of `ramshield-types` = analytics, protocol, storage, enforcement, config, forecasting, detection (+root). `cgnat`, `metrics`, `xdp`, `mesh`, `ramwal` do not depend on it.
**Status: FIXED.** The README now names the eight crates that read it - seven plus the root package - and the five that do not. The test checks the list against the workspace manifests.


**F3 (MEDIUM)** `crates/ramshield-types/README.md:170`
> "this crate has no logic, only types."

Code: `src/error.rs:36-71` reason string mapping, `src/ip_network.rs:39-72` CIDR normalization, `src/util.rs:20-25` eviction on push.
**Status: FIXED.** The README now says the crate is not type-only: it owns the reason-string mapping, the CIDR normalisation and validation, and the ring buffer's eviction.


**F4 (MEDIUM)** `crates/ramshield-types/README.md:120`
> "Used for status code history, event ring buffers, and block logs."

Code: repo-wide grep `BoundedVecDeque` → hits only `crates/ramshield-types/src/util.rs`. Zero users.
**Status: FIXED.** The README now says the type has no user in the workspace, where the old README claimed three uses.


**F5 (MEDIUM)** `crates/ramshield-types/README.md:162`
> "IP networks can't be `DashMap` keys directly (no `Hash` impl for `IpAddr + prefix_len`)."

Code: `src/ip_network.rs:8` — `#[derive(… Eq, Hash, Serialize)] pub struct IpNetwork`. Hash exists.
**Status: FIXED.** The README now says `IpNetwork` derives `Hash` and can key a map directly, and that `pack()` is a separate `u128` form which is not on a production path.


---

## Sub-claim 2 — BlockReason documentation

**F6 (CRITICAL)** `crates/ramshield-types/README.md:89-98`
> ```rust
> pub enum BlockReason { RateLimit, SubnetBurst, PulseWave, Manual }
> ```
> "Maps to stable strings … `"rate_limit"`, `"subnet_burst"`, `"pulse_wave"`, `"manual"`."

Code: `src/error.rs:25-34` has **8** variants — `HighRps, SubnetBatch, ForecastAnomaly, EntropyAnomaly, ManualBlock, L7HighRps, L7Cost, Http2StreamAbuse`; `as_str()` (`error.rs:38-49`) returns `high_rps, subnet_burst, forecast_anomaly, entropy_anomaly, manual, l7_high_rps, l7_cost, http2_stream_abuse`. Grep for `rate_limit|pulse_wave` across repo: hits only three READMEs — no code emits those tokens; `from_reason_str` (`error.rs:52-70`) never accepts them. 4 of 8 real reasons missing; 2 documented tokens non-existent. Wire vocabulary + Prometheus labels wrong.

**Status: FIXED.** The README lists all eight variants and the eight
tokens `as_str()` returns.

**F7 (MEDIUM)** `crates/ramshield-metrics/README.md:61`
> `pub reason: String,    // "rate_limit", "subnet_burst", "pulse_wave", "manual"`

Same defect as F6 (`src/types.rs:21-26` confirms field exists, comment enumerates wrong tokens).

**F8 (MEDIUM)** `crates/ramshield-forecasting/README.md:106`
> "**Internal dependencies:** `ramshield-types` (`EnforceCommand`, `BlockReason::RateLimit`)."

Code: grep `BlockReason` in `crates/ramshield-forecasting/src` → no hits. Variant `RateLimit` does not exist.

**Cleared:** `crates/ramshield-enforcement/README.md:104-117` — BlockReason list and all 8 wire strings match `error.rs:25-49` exactly.

---

## Sub-claim 3 — config schema examples

**F9 (MEDIUM)** `crates/ramshield-config/README.md:73-86` — template keys `window_seconds`, `batch_window_seconds`, `batch_threshold_rps`, `event_threshold_rps` do not exist. Code (`src/sections/detection.rs:4-92`): `rate_window_secs`, `batch_window_ms`, `subnet_batch_threshold`, `subnet_batch_min_events`; no event-threshold concept at all.

**Status: FIXED.** The template's detection keys were `window_seconds`,
`batch_window_seconds`, `batch_threshold_rps` and `event_threshold_rps`;
none of them exists. The template now uses the real ones
(`rate_window_secs`, `batch_window_ms`, `subnet_batch_threshold`,
`subnet_batch_min_events`, `promote_min_events`, `pulse_window_secs`,
`pulse_threshold_samples`) and the `l7_*` names the code reads.

**F10 (MEDIUM)** `crates/ramshield-config/README.md:88-94`
> `[l7.rule]` / `cost_weight`, `baseline_latency_us`, `rps_threshold`, `http2_min_streams`, `http2_reset_ratio_pct`, `block_ttl_secs`

Code: `src/lib.rs:32-57` — no top-level `l7` section; L7 lives under `detection` (`detection.rs:77-92` `l7_*` scalars, `l7_rules: Vec<L7Rule>`), and `L7Rule` (`detection.rs:96-115`) has `route_hash, method, max_rps, max_latency_us, cost_weight, baseline_latency_us, max_effective_rps` — no `rp

s_threshold`/`http2_min_streams`/`http2_reset_ratio_pct`/`block_ttl_secs`, and `route_hash` is required (non-zero, `validate.rs:102-104`), so the template cannot load.

**Status: FIXED.** `[l7.rule]` is not a section. L7 is `detection.l7_rules`, an array
of tables, and `L7Rule` requires `route_hash`, `cost_weight` and
`baseline_latency_us`. The template now shows the commented
`[[detection.l7_rules]]` shape.

**F11 (MEDIUM)** `crates/ramshield-config/README.md:117`
> `durabil

ity = "hard"`

Code: `src/error.rs:73-80` — `Durability { None, Flush, Fsync, GroupCommit }`, serde default naming → valid tokens are `None|Flush|Fsync|GroupCommit`.

**Status: FIXED.** `durability = "hard"` is not a value. The enum is
`None | Flush | Fsync | GroupCommit`, and `GroupCommit` is the default.

**F12 (MEDIUM)** `crates/ramshield-config/README.md:103` and `:122`
> `[ipc] listen_addr` / `[dashboard] listen_addr`

Code: `src/sections/ipc.rs:5` `tcp_addr`; `src/sections/dashboard.rs:9` `http_addr`.

**Status: FIXED.** `[ipc] listen_addr` is `tcp_addr`, `[dashboard] listen_addr` is
`http_addr`.

**F13 (MEDIUM)** `crates/ramshield-config/README.md:126`
> `[mesh] node_id = "default"`

Co

de: `src/sections/security.rs:161` `node_id: u32` → string value is a parse error.

**Status: FIXED.** `mesh.node_id` is a `u32`, not the string `"default"`.

**F14 (MEDIUM)** `crates/ramshield-config/README.md:110-111`
> `[forecasting]` / `enabled = true`

Code: `src/sections/forecasting.rs:4-11` — every field (`ewma_alpha, hw_beta, hw_gamma, seasonality_period, anomaly_zscore, min_entropy`) lacks `#[serde(default)]`; a present-but-partial `[forecasting]` table fails to deserialize.

**Status: FIXED.** `[forecasting]` has an `enabled` field, but none of its fields has
a `serde(default)`, so all seven must be present once the section
appears. The old template's two-line section would not have loaded
at all.

**F15 (MEDIUM)** `crates/ramshield-config/README.md:26`
> `RAMSHIELD_L7_RULE__COST_WEIGHT=8.0`

Code: `src/env.rs` env-var list has `RAMSHIELD_DETECTION__L7_*` only; no `RAMSHIELD_L7_RULE__*` handler exists.

**Status: FIXED.** `RAMSHIELD_L7_RULE__COST_WEIGHT` does not exist. The environment
variable for an L7 setting is `RAMSHIELD_DETECTION__L7_ENABLED`.

**F16 (MEDIUM)** `crates/ramshield-config/README.md:33-38` — validation list wrong. Code `src/valida

te.rs`: no event-threshold comparison (no such field), no XDP-interface-name length check, no "placeholder value" rejection (only `key_id:hex` shape/hex-charset/≥32-char/dup-id, `validate.rs:403-434`), and public dashboard bind requires `dashboard.tls_enabled=true` **assertion** + `admin_password_hash` (`validate.rs:366-380`) — not "HTTPS" enforcement; `exposure_warnings()` (`validate.rs:443-475`) states the stack has no TLS.

**Status: FIXED.** The validation list was wrong in every entry: there is no
"batch thresholds > event thresholds" rule, no "XDP interface name
length >= 2" rule and no placeholder-HMAC rule. The README now lists
the checks `src/validate.rs` actually enforces.

**F17 (MEDIUM)** `crates/ramshield-config/README.md:43-45` and `:175`
> `let cfg = handle.get();` / `handle.set(new_config);` / `Config::load(".")`

Code: `src/lib.rs:20` `pub type ConfigHandle = Arc<ArcSwap<Config>>;` — `ArcSwap` exposes `load`/`store`, not `get`/`set`; `src/lib.rs:74` `load(path)` reads a **file** (a directory path fails).

**Status: FIXED.** `ConfigHandle` is an `Arc<ArcSwap<Config>>`, so it is `load()` and
`store()`, not `get()` and `set()`; and `Config::load` takes the path
of a file, not a directory.

**F18 (MEDIUM)** `crates/ramshield-config/README.md:212`
> "No internal workspace crate dependencies"

Code: `crates/ramshield-config/Cargo.toml:8` → `ramshield-types = { path = "../ramshield-types" }`.

**Status: FIXED.** The crate does depend on a workspace crate: `ramshield-types`, for
`Durability` and `HttpMethod`.

**F19 (MEDIUM)** `crates/ramshield-config/README.md:215`
> "75 fuzz tests in `tests/fuzz.rs`"

Code: `grep -c '#\[test\]' crates/ramshield-config/tests/fuzz.rs` → **4** (2 proptest + 2 unit).

What changed, and against what:

- **F9** — the template's detection keys were `window_seconds`,
  `batch_window_seconds`, `batch_threshold_rps` and `event_threshold_rps`;
  none of them exists. The template now uses the real ones
  (`rate_window_secs`, `batch_window_ms`, `subnet_batch_threshold`,
  `subnet_batch_min_events`, `promote_min_events`, `pulse_window_secs`,
  `pulse_threshold_samples`) and the `l7_*` names the code reads.
- **F10** — `[l7.rule]` is not a section. L7 is `detection.l7_rules`, an
  array of tables, and `L7Rule` requires `route_hash`, `cost_weight` and
  `baseline_latency_us`. The template now shows the commented
  `[[detection.l7_rules]]` shape.
- **F11** — `durability = "hard"` is not a value. The enum is
  `None | Flush | Fsync | GroupCommit`, and `GroupCommit` is the default.
- **F12** — `[ipc] listen_addr` is `tcp_addr`, `[dashboard] listen_addr`
  is `http_addr`.
- **F13** — `mesh.node_id` is a `u32`, not the string `"default"`.
- **F14** — `[forecasting]` has an `enabled` field, but none of its
  fields has a `serde(default)`, so all seven must be present once the
  section appears. The old template's two-line section would not have
  loaded at all.
- **F15** — `RAMSHIELD_L7_RULE__COST_WEIGHT` does not exist. The
  environment variable for an L7 setting is
  `RAMSHIELD_DETECTION__L7_ENABLED`.
- **F16** — the validation list was wrong in every entry: there is no
  "batch thresholds > event thresholds" rule, no "XDP interface name
  length >= 2" rule and no placeholder-HMAC rule. The README now lists
  the checks `src/validate.rs` actually makes.
- **F17** — `ConfigHandle` is an `Arc<ArcSwap<Config>>`, so it is
  `load()` and `store()`, not `get()` and `set()`; and `Config::load`
  takes the path of a file, not a directory.
- **F18** — the crate does depend on a workspace crate:
  `ramshield-types`, for `Durability` and `HttpMethod`.
- **F19** — 4 property-based tests, not 75.

`crates/ramshield-config/tests/readme_template.rs` extracts the ```toml
block from the README and loads it through `Config::from_toml_file`,
which parses and validates. The old template fails it - the phantom
`[l7.rule]` section is a TOML parse error - and the corrected one
passes, so the drift cannot come back unnoticed.

---

## Sub-claim 4 — analytics / Welford

**Status: FIXED.** There are 4 property-based tests in `tests/fuzz.rs`, not 75.

**F20 (MEDIUM)** `crates/ramshield-analytics/README.md:5` and `:30`
> "replaces unbounded `HashSet<IpAddr>` detection" / "replace unbounded `HashSet<IpAddr>` in the detection pipeline"

Code: repo grep `ramshield_analytics` → only `benches/hot_paths.rs:396,405` and the crate itself; `crates/ramshield-cgnat` mentions it in comments only. No crate depends on `ramshield-analytics` except root benches; detection does not use it, and no `HashSet<IpAddr>` exists in detection.

**F21 (MEDIUM)** `crates/ramshield-analytics/README.md:15` (and `src/cms.rs:3`, `src/lib.rs:4`)
> "**Memory:** 512 KiB (4 rows × 65,536 columns × AtomicU64)"

Code: `src/cms.rs:11-12` `CMS_ROWS=4`, `CMS_COLS=65_536`, `counts: Vec<AtomicU64>` → 4×65 536×8 B = **2 MiB**. Off 4×.

**F22 (MEDIUM)** `crates/ramshield-analytics/README.md:19-20`
> "Single shared instance per daemon" / "Used for identifying top-k threat sources"

Code: no instance in any daemon (F20 grep); `src/cms.rs` API = `increment/estimate/decay/inserted_since_decay` only — no heavy-hitter/top-k query.

**F23 (MEDIUM)** `crates/ramshield-analytics/README.md:33`
> "**Deterministic convergence** with proper decay scheduling"

Code: grep `cms.decay()` → single hit `src/cms.rs:104` (unit test). No scheduler calls decay.

**F24 (MEDIUM)** `crates/ramshield-analytics/README.md:37-50` usage snippet
> `use ramshield_analytics::{SubnetHll, DecayingCountMinSketch, Welford};` / `let hll = SubnetHll::default(); hll.insert(hash_value);` / `cms.insert(ip_hash, expiry_ms);`

Code: `src/lib.rs:8-10` exposes only `pub mod cms/hll/welford` — no crate-root re-exports (import path invalid); `src/hll.rs:24` `pub fn insert(&mut self, hash: u64)` — snippet's `hll` is not `mut`; `src/cms.rs` has no `insert` (it is `increment`).

**F25 (LOW)** `crates/ramshield-analytics/README.md:23`
> "Exponentially Weighted Welford — **Memory**: 32 bytes"

Code: `src/welford.rs:20-25` — `AtomicU32` + `AtomicU32` + `AtomicU64` + `f32` = 20 B → 24 B padded, not 32.

---

## Sub-claim 5 — XDP hardware/kernel verification limits

**F26 (MEDIUM)** `crates/ramshield-xdp/README.md:7` and `:17`
> "BPF compilation requires `clang` and `llvm-strip` at build time" / "`build.rs` compiles the BPF source, strips it"

Code: `crates/ramshield-xdp/build.rs:24-31,35-55,57-99` — `bpf-linker` + `cargo build --target=bpfel-unknown-none`; no clang, no llvm-strip, no strip step. Wrong build prerequisites.

**Status: FIXED.** The README now says the BPF program is Rust
(`ramshield-xdp-bpf/src/main.rs`, built with `aya-ebpf`), and that the
build needs `bpf-linker` and the `bpfel-unknown-none` target — not
`clang` and `llvm-strip`, and with no strip step. `build.rs` looks for
`bpf-linker` on `PATH` and in `~/.local/bin`; when it finds neither, the
build **fails** - `build.rs` panics with the install step and writes no
placeholder ELF.

**F27 (MEDIUM)** `crates/ramshield-xdp/README.md:35`
> "The enforcement crate also defines the BPF map types (`BlocklistKey`, `BlocklistValue`)"

Code: grep `BlocklistValue` repo-wide → only the comment `crates/ramshield-xdp/src/lib.rs:5`. No such type exists.

**Status: FIXED.** There is no `BlocklistValue` type in the tree — the
map value is a plain `u64`, the absolute expiry on the monotonic clock,
with `u64::MAX` for permanent. The README now says that.

**F28 (MEDIUM)** `crates/ramshield-xdp/README.md:41-44`
> "2. Hashes it into the blocklist map. 3. If the IP is in the map, drops the packet."

Code: `crates/ramshield-xdp/ramshield-xdp-bpf/src/main.rs:200-215` copies raw wire bytes into the key (no hash); `:220-228` drops only if `now < expiry` (expired entry passes); `:186` and `:230` drop via LPM CIDR maps `BLOCKCIDR6`/`BLOCKCIDR` even when the IP is absent from the flat map. Also VLAN stripping `:141-154`. Documented behaviour omits expiry semantics and the CIDR drop path.

**Status: FIXED.** The README now describes the drop path as the code
does it: the key is the raw wire bytes of the address (no hash), the
flat-map hit only drops while its expiry is in the future, and the CIDR
LPM tries (`BLOCKCIDR` / `BLOCKCIDR6`) drop a packet even when the
address is absent from the flat map.

**F29 (MEDIUM)** `crates/ramshield-xdp/README.md:68`
> "No unit tests — the crate's correctness is verified by `ramshield-enforcement` integration tests that load the BPF program"

Code: `crates/ramshield-xdp/src/lib.rs` has **19** unit tests (v4/v6 key layout, wire-contract v4/v6, split/routing, VLAN, BLOCKLIST/BLOCKLIST6 separation) plus `#[ignore]`-marked root-required tests: `aya_load_verifies_maps_and_program` (`:376-378`) and `insert_lookup_roundtrip_v4` (`:395-396`) — these are exactly the live-kernel/hardware tests the backlog item asks to be identified. (Root-required test set: these two + `insert_lookup_roundtrip_v6`; `crates/ramshield-xdp/tests/ddos_stress.rs` also needs root.)

**F30 (MEDIUM)** `crates/ramshield-xdp/README.md:5` — "compiles a BPF program (written in C or Rust)": build.rs compiles `ramshield-xdp-bpf/src/main.rs` (Rust/aya-ebpf). `bpf/main.rs` is a 7-line stub not built; no C source in the tree. Low impact, folded here. **Status: FIXED** — the README now says the BPF program is Rust.

---

## Sub-claim 6 — forecast priors / polling

**F31 (MEDIUM)** `crates/ramshield-forecasting/README.md:25`
> "`Forecaster::tick()` runs periodically (default: every 10 seconds). It pulls the last 120 batch records from `Store::get_batch_history()`"

Code: `src/forecaster.rs:76` `interval(Duration::from_secs(1))` — **1 Hz**; `src/lib.rs:129` "tick_hw (1 Hz)"; `src/validate.rs:7` "Forecaster ticks at 1 Hz". Method is `tick_hw` (`forecaster.rs:84`), no `tick()`. `get_batch_history` exists only in `crates/ramshield-metrics/src/history.rs:84` and `src/engine/mod.rs:316` — not on `Store`, and the forecaster reads `store.traffic.events_last_second` / `unique_ips_window` (`forecaster.rs:85-87`). Diagram `README.md:4` repeats the wrong source.

**Status: FIXED.** The README now names `Forecaster::run()` (the public loop, which calls the crate-private `tick_hw`), the diagram starts at `store.traffic`, the tick-cycle paragraph names the counters the code reads, and the "Reads from" line names `Store::traffic` instead of `Store::get_batch_history()`. The quote above is the text that was wrong.

**F32 (MEDIUM)** `crates/ramshield-forecasting/README.md:44` and `:110`
> `period: usize,  // default: 120 ticks = 20 minutes` / "The Holt-Winters update is O(period) where period defaults to 120"

Code: `src/forecaster.rs:54-59` passes `config.seasonality_period`; default **3 600** (`crates/ramshield-config/src/sections/forecasting.rs:21`). `src/models.rs:20-40` updates one seasonal slot → O(1), not O(period).

**F33 (MEDIUM)** `crates/ramshield-forecasting/README.md:54` and `:56-62`
> "operates on the *batch-level* event rate (events per 500ms window)" / `CusumState { cusum_value, baseline, warmup_samples, threshold }`

Code: `src/models.rs:93-98` `update(&mut self, z: f64)` — "Feed a z-score (already normalized by sigma)"; called with the HW residual z (`forecaster.rs:102`). Real fields `src/lib.rs:83-88`: `s_upper, s_lower, k, h` (no warmup, no baseline, no threshold field).

**F34 (MEDIUM)** `crates/ramshield-forecasting/README.md:80`
> "exceeds a confidence threshold (default 0.95)" / "The model uses pre-computed likelihood tables"

Code: `src/hypothesis.rs:21-23` → `threshold: 0.75`, `cold_threshold: 0.85`, `cold_ticks: 60`. Likelihoods are hand-written functions `src/hypothesis.rs:120-247` — no tables.

**F35 (MEDIUM)** `crates/ramshield-forecasting/README.md:86`
> "maintains the top-N event rates seen in the current window. Used to compute percentiles for the dashboard"

Code: `src/forecaster.rs:96-97` pushes `residual.abs()`; `src/lib.rs:15-16` "Empirical quantile of forecast residuals (legacy, transitional)"; used as the legacy spot-alarm threshold `forecaster.rs:232-242`. Not event rates, not dashboard percentiles.

**F36 (MEDIUM)** `crates/ramshield-forecasting/README.md:104`
> "**Written to:** … `ramshield-metrics` (forecast values, z-scores, hypothesis posteriors)"

Code: grep `posterior` in `crates/ramshield-metrics/src` → no hits. Metrics exposes only `set_forecast_hw` (`history.rs:67`) and `set_entropy` (`history.rs:75`).

**F37 (MEDIUM)** `crates/ramshield-forecasting/README.md:114`
> "CUSUM warmup period (6 samples)"

Code: `CUSUM_WARMUP_SAMPLES = 6` is in `crates/ramshield-detection/src/rate_tracker.rs:18` (detection crate). Forecasting `CusumState` has no warmup; `src/models.rs:82-104`.

---

## Sub-claim 7 — module-level status claims

**F38 (MEDIUM)** `crates/ramshield-storage/README.md:46-53` — `IpRecord { event_count, first_seen_ns, last_seen_ns, threat_score, block_state, status_history: BoundedVecDeque<u16> }`. Code `crates/ramshield-storage/src/record.rs:129-165`: `ip, request_count, ewma_rps, cusum_s, baseline_rps, prev_sample_hot, sample_count, relative_breach_streak, pulse_samples_in_window, pulse_window_start_ns, first_seen_ns, last_seen_ns, bytes_in, status_dist[5], proto_fingerprint, threat_score, block_state` — no `event_count`, no `status_history`.

**F39 (MEDIUM)** `crates/ramshield-storage/README.md:56`
> "IPs with fewer than 64 events store their status history in the struct itself … Above 64 events, the history spills to a `BoundedVecDeque` on the heap."

Code: `src/record.rs:96-120` — `Value::Inline`/`Value::Blob` byte threshold `INLINE_MAX`, not an event count; no `BoundedVecDeque` in the storage crate at all (F4 grep).

**F40 (MEDIUM)** `crates/ramshield-storage/README.md:65`
> "The subnet key is a `u128` that packs the network address and prefix length … `subnet_key_v4(octets)` zeros the host bits and shifts the result into the upper 32 bits"

Code: `src/subnet.rs:9-11` `pub fn subnet_key_v4(octets: [u8;4]) -> u32` (not u128); `:29` placed in the **low** 32 bits; prefix length is not packed at all.

**F41 (MEDIUM)** `crates/ramshield-storage/README.md:69-72`
> "eviction runs in two passes: 1. Expired entries … 2. **LRU eviction** — if still over limit, the least-recently-seen entries are removed in batches of 1024."

Code: grep `fn evict` in `crates/ramshield-storage/src` → `store/capacity.rs:23 evict_batch(&self, keys: &[IpAddr])` (caller-supplied keys) and `store/capacity.rs:58 evict_expired(&self)` (TTL-expired only). No LRU pass, no 1024-batch eviction in the crate.

**F42 (MEDIUM)** `crates/ramshield-storage/README.md:84` and `:93-96`
> `durability: Durability,  // NoSync | Fsync | Fdatasync` / "The WAL supports three durability modes"

Code: `crates/ramshield-types/src/error.rs:73-80` → `None | Flush | Fsync | GroupCommit` (four).

**F43 (MEDIUM)** `crates/ramshield-storage/README.md:103-113`
> `TrafficCounters { ram_limit_mb, ram_bytes, total_events, unique_ips, uptime_secs, threat_samples: Mutex<Vec<(IpAddr,f32)>> }` / "No locks in the hot path … `threat_samples` mutex is only touched during batch flush (every 500ms)."

Code: `src/record.rs:5-24` → `events_last_second, unique_ips_window, promoted_ips, subnet_window[AtomicU64;256], threat_sample: SegQueue<(IpAddr,f32)>, ram_limit_mb, used_bytes, uptime_secs`. No `ram_bytes`, no `total_events`, no mutex; flush cadence is `detection.batch_window_ms`, not 500 ms.

**F44 (MEDIUM)** `crates/ramshield-storage/README.md:141`
> "RAM limit enforcement (insert until limit hit, **verify eviction**)"

Code: same as F41 — the store has no LRU eviction to verify (see F41 grep). (Insert-path rejection behaviour not independently re-verified this pass.)

**F45 (MEDIUM)** `crates/ramshield-metrics/README.md:30`
> `batch_history: Mutex<VecDeque<BatchRecord>>,  // last 120 batch stats`

Code: `crates/ramshield-metrics/src/lib.rs:25` `const HISTORY: usize = 80;`.

**F46 (MEDIUM)** `crates/ramshield-metrics/README.md:39`
> "The forecaster reads these to compute Holt-Winters forecasts"

Code: `crates/ramshield-forecasting/src/forecaster.rs:85-87` reads `store.traffic` counters, never `Metrics::get_batch_history` (`history.rs:84`).

**F47 (MEDIUM)** `crates/ramshield-metrics/README.md:41-50` — `BatchRecord { timestamp_ms, total_events, unique_ips, rps_estimate, blocked_new, subnet_blocked, cold_skip_pct }`. Code `crates/ramshield-metrics/src/types.rs:4-18`: `ts_ms, events, unique_ips, promoted, cold_skipped, promoted_events, cold_skipped_events, blocks, hot_subnets`.

**F48 (MEDIUM)** `crates/ramshield-metrics/README.md:71-85` — `DashboardSnapshot` field list. Code `crates/ramshield-metrics/src/types.rs:48-88`: `ts_ms, uptime_secs, ips_tracked, blocked_total, ram_bytes, ram_limit_mb, ram_pct, cpu_usage, memory_usage_mb, total_ram_mb, ipc_requests, events_ingested, events_rejected, frames_rejected_total, channel_depth, events_shed, batches_total, promotions, cold_skipped, blocks_applied, pipeline, is_healthy, health_reason, xdp_active, wal_lsn, pending_expirations, xdp_apply_failures, xdp_projection_stale, xdp_configured, protection_state`. No `total_events`, `blocked_ips`, `rps_current`, `forecast_hw`, `forecast_z`, `entropy`, `batch_history`, `block_log`, `subnets`, `modules`.

**F49 (MEDIUM)** `crates/ramshield-metrics/README.md:25-32` — `Metrics { requests, blocks, ingested, rejected, batch_history, block_log }`. Code `crates/ramshield-metrics/src/lib.rs:28-30`: `requests_total`, `blocks_total`, `events_ingested`, `events_rejected` (plus ~40 more fields).

**F50 (MEDIUM)** Phantom-crate callers across READMEs: `crates/ramshield-config/README.md:232-238` (`ramshield-engine`, `ramshield-ipc`, `ramshield-wal`, `ramshield-dashboard`), `crates/ramshield-metrics/README.md:167,169` (`ramshield-engine`, `ramshield-dashboard`, `ramshield-ipc`), `crates/ramshield-storage/README.md:129` (`ramshield-dashboard`), `crates/ramshield-enforcement/README.md:139` (`ramshield-dashboard`, `ramshield-metrics`). Code: workspace members `Cargo.toml:2-16` = root + config, detection, enforcement, forecasting, metrics, protocol, storage, types, xdp, cgnat, analytics, mesh, ramwal — **no** engine/ipc/wal/dashboard crates exist.

**F51 (LOW)** `crates/ramshield-types/README.md:24` — "Nine workspace crates need to share" → 7 crates actually depend on `ramshield-types` (F2 grep).
**Status: FIXED.** The README now says eight crates - seven plus the root package - and names them.


**F52 (LOW)** `crates/ramshield-forecasting/README.md:50` + `:98` + `src/lib.rs:69` — "O(1) memory (3 floats)", "not part of the public API", "3 floats = 24 bytes". Code: `src/lib.rs:71-76` `EwmAVar { ewma: f64, var_ewma: f64, count: u64, alpha: f64 }` = 32 B; `pub struct` → public type.

**F53 (LOW)** `crates/ramshield-metrics/README.md:177` — "14 tests" → 16 static `#[test]` markers in the crate.

**F54 (LOW)** `crates/ramshield-storage/README.md:141` — "38 tests" → 42 static `#[test]` markers in the crate.

---

## Cleared

- `crates/ramshield-enforcement/README.md:104-117` — BlockReason variants + all 8 wire strings match `crates/ramshield-types/src/error.rs:25-49`.
- `crates/ramshield-config/README.md:24` and `:75` — `RAMSHIELD_ENGINE__RAM_LIMIT_MB`, `RAMSHIELD_DETECTION__RPS_THRESHOLD` exist (`src/env.rs:41,57`).
- `crates/ramshield-config/README.md:33` — "RAM limits >= 64 MB" matches `src/validate.rs:22-24`.
- `crates/ramshield-forecasting/README.md:69-70` — four hypotheses exist (`src/lib.rs:94-99`); only the H1/H3 labelling is wrong (F2 below).
- `crates/ramshield-analytics/README.md:10` — HLL 1024 bytes matches `src/hll.rs:8` (`registers: [u8; 1024]`).
- `crates/ramshield-xdp/README.md:60` — enforcement is the only runtime consumer of `BPF_ELF`; `crates/ramshield-enforcement/src/xdp.rs:32` defines `BlocklistKey(pub [u8;16])` as claimed.

---

## Worst three

1. **F6 (CRITICAL)** — `crates/ramshield-types/README.md:89-98`: BlockReason variant list and wire/Prometheus tokens wrong (4 of 8 reasons missing; `rate_limit`/`pulse_wave` emitted by nothing).
2. **F1 (CRITICAL)** — `crates/ramshield-forecasting/README.md:72-78`: hypothesis→index map inverted. Code: `src/lib.rs:94-105` `Normal=0, VolumetricDoS=1, SlowRampDoS=2, FlashCrowd=3`; README claims `FlashCrowd=H1`, `Volumetric=H3`. Code blocks on H1 (`src/forecaster.rs:177-183` → `preemptive_block`) and explicitly does **not** block on H3 (`:211-217`). README says the opposite — an operator reading it concludes volumetric floods are not blocked and flash crowds are.
3. **F28 (MEDIUM)** — `crates/ramshield-xdp/README.md:41-44`: documented BPF drop behaviour omits the expiry check and the CIDR LPM drop path, and says the IP is "hashed" into the map (`bpf/main.rs:176-236`).

---

## NOT CHECKED (no verdict issued)

- Enforcement `XdpApplier` trait signature vs `README.md:91-95`; enforcement crate dependency list vs `README.md:135`.
- `crates/ramshield-xdp/README.md:52` fail-open/fallback wording vs `src/engine/boot.rs` XDP attach error path.
- `crates/ramshield-metrics/README.md:155` `get_system_usage()` return tuple / `/proc` claim (src/system.rs not re-read this pass).
- `crates/ramshield-analytics/README.md:55` BENCHMARK_BASELINES content (file exists; analytics entries unverified).
- `crates/ramshield-cgnat/README.md` shared-memory table claims (publish/read parity, `ShmTableManager::publish_rule`) — not re-verified this pass.
- `crates/ramshield-detection/README.md` doc cross-references (`docs/OPERATIONS.md`, `docs/DEFENSE_MODEL.md`) and each signal-family sentence against `crates/ramshield-detection/src/`.
- Bench-filter names in `benches/hot_paths.rs` (README `store_insert`/`store_lookup`/`subnet_key`, `ip_network_pack`, `bounded_vec_deque`).
- Whether `Config::load()` already applies env overrides (README API example applies them again).
- Whether detection and forecasting share one enforcement channel (`src/engine/` channel wiring).

**F55 (P1)** `crates/ramshield-types/README.md:155-161`
> `pub enum Durability { NoSync, Fsync, Fdatasync }` and "Controls how `Wal::append()` syncs to disk. `NoSync` for benchmarking, `Fdatasync` for production, `Fsync` for financial environments."

Code: `crates/ramshield-types/src/error.rs:73-80` — the enum is `None | Flush | Fsync | GroupCommit`, with `GroupCommit` the `#[default]`. `NoSync` and `Fdatasync` are not variants, so an operator who copies them into `wal.durability` gets a config parse error - and the config crate's README, which was corrected, now names the real tokens, so the two READMEs contradict each other.

Replacement: `pub enum Durability { None, Flush, Fsync, GroupCommit }` and "Controls how `Wal::append()` syncs to disk. `None` for benchmarking, `Flush` for production, `Fsync` where every block must survive power loss, `GroupCommit` (the default) for group-committed `fsync`."
**Status: FIXED.** The README now shows the real enum and maps each variant to what it does. It also says the shipped configuration default is `Flush`, which the enum's `#[default]` is not.


**F56 (P2)** `crates/ramshield-types/README.md:131-140` — the `RsError` variant list.

Code: `crates/ramshield-types/src/error.rs` — the real variants are `NotFound(String)`, `CapacityExceeded { limit_mb }`, `Serde(String)`, `Io(std::io::Error)`, `CorruptWal { offset }` and `RecordTooLarge { size, max }`.
**Status: FIXED.** The README now shows the real variants, and says that `RsError` is the storage crate's error type rather than the whole system's.


**F57 (P2)** `crates/ramshield-types/README.md:49-60` — the `EnforceCommand` field list.

Code: `crates/ramshield-types/src/command.rs:23-39` — the real fields are `decision_id: Uuid`, `policy_version: u64`, `source: String`, `actor: String`, `timestamp_utc: i64`, `ttl_seconds: u64`, `reason: String`, `ip: IpAddr`, `cidr: Option<IpNetwork>`, `action: EnforceAction`, `evidence_source: EvidenceSource`. There is no `id` and no `source_ip`/`source_port`, and `reason` is a `String`.
**Status: FIXED.** The README now shows the real fields, says that `cidr` is the one `Option`, and says that `reason` is a wire string rather than a `BlockReason`.


**F58 (P2)** `crates/ramshield-types/README.md:77-83` — the `EnforceResult` field list.

Code: `crates/ramshield-types/src/command.rs:48-56` — the real fields are `decision_id: Uuid`, `committed: bool`, `applied: bool`, `wal_lsn: Option<u64>`, `xdp_applied: bool`, `error: Option<String>`.
**Status: FIXED.** The README now shows the real fields, and says what `committed`, `applied` and `xdp_applied` each mean - they can disagree.


**F59 (P2)** `crates/ramshield-types/README.md:30-36` — the `ConnectionEvent` field list.

Code: `crates/ramshield-types/src/events.rs:65-75` — the real fields are `ip: IpAddr`, `timestamp_ns: u64`, `bytes: u64`, `status_code: u16`, `proto_fingerprint: u32`, `l7: Option<L7Metadata>`.
**Status: FIXED.** The README now shows the real fields, and says that `l7` is optional so the legacy L3/L4 contract stays wire-compatible.


**F60 (P2)** `crates/ramshield-types/README.md:142-149` — the `EnforcementError` variant list.

Code: `crates/ramshield-types/src/command.rs:58-69` — the real variants are `Wal(String)`, `Storage(String)`, `Xdp(String)`, `Duplicate(Uuid)` and `InvalidCommand(String)`. There is no `AlreadyProcessed` and no `Deduplicated`.
**Status: FIXED.** The README now shows the real variants, and says that a repeated decision id returns the original result rather than an error.


**F61 (P3)** `crates/ramshield-types/README.md:121-124` — the `BoundedVecDeque` field list.

Code: the real struct has a public `cap: usize` and a private `inner`.
**Status: FIXED.** The README no longer shows a field list. It names the type's path, says that `cap` is a public field, and says that no crate in the workspace uses the type.


**F62 (P2)** `crates/ramshield-xdp/README.md:7` — the claim that the BPF build degrades gracefully.

Code: `crates/ramshield-xdp/build.rs:26-31` — `build.rs` **panics** when `bpf-linker` is missing ("XDP BPF build failed: bpf-linker missing ... No placeholder ELF is written."). The README must not promise a soft skip the build does not do.
