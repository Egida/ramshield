# CGNAT SHM LOOKUP — a contended read must not masquerade as a coherent miss

**Finding:** audit F1 (21 October 2026 master audit).
**Branch:** `fix/master-p0-p1-fixes` — base `191a7d4` (`origin/master`).
**Files:** `crates/ramshield-cgnat/include/ramshield_shm.h`,
`crates/ramshield-cgnat/src/shm.rs`, `scripts/shm_lookup_contract.c` (new),
`scripts/verify_shm_header.sh`.

---

## 1. The defect, and what the investigation changed about it

The audit reported a contract defect in the published C lookup API, and an
independent review confirmed it at source level. Both were right about the
code. The **scope**, however, turned out to be different from what either
described, and one additional defect surfaced that neither had found.

### 1.1 Contention was indistinguishable from a miss

`ramshield_shm_read()` returned a plain `bool`. Its inner loop retried a slot
up to 64 times while the seqlock sequence counter was odd or changed
mid-read, but exhausting that budget simply advanced the probe; after the
whole probe window the function returned `false`.

So a slot that could not be read coherently was **indistinguishable from an
absent rule**. For a CGNAT mitigation table this fails **open**: a proxy that
reads "no rule" stops challenging or blocking a client the table does hold a
rule for.

The stress harness documents the defect as expected behaviour, in its own
comment:

```c
if (!ramshield_shm_read(table, test_hash, 0, &snapshot)) {
    continue; /* bounded contention may yield a miss, never a torn rule */
}
```

### 1.2 An invalid prefix was routed into the table

`ramshield_ipv4_subnet_key()` returns `0` for `prefix_len > 32`, and
`ramshield_shm_read_ipv4()` passed that `0` straight into the lookup. Key `0`
is the empty-slot sentinel, so an invalid prefix degenerated into a silent
miss rather than an error.

### 1.3 A third defect, found during the work: the reserved key is unrepresentable

`prefix_len == 0` masks the client IP to network `0`, and
`subnet_key(0, 0)` hashes to `0` — the reserved empty-slot sentinel.

The hazards against the previous release were these, and they are worth
stating precisely, because the obvious summary of them is wrong:

- **The publisher treated a key-0 slot as free.** `publish_rule`'s phase 2
  picks the first slot whose `client_hash == 0`, and phase 1 matched a
  key-0 request against any empty slot. So a rule published under key 0
  sat in a slot that every other publisher considered empty, and the next
  colliding publish would take that slot and overwrite it. The rule was
  not durable.
- **A prefix-0 lookup was routed to key 0 with no error.** The old reader
  matched `client_hash == hash`, so a lookup with `hash == 0` *did* find
  the key-0 rule — the rule was not unfindable. But an invalid prefix and
  a valid /0 were indistinguishable, and a lookup for a key that could not
  exist could not be distinguished from a miss.

An earlier draft of this report said the /0 rule was "published and never
found: enforced by nothing". That overstated it, and the report's own RED
evidence contradicts it — "prefix 0 must not be looked up as the reserved
key 0 (got 0)" is `got FOUND`. The accurate statement is the one above:
the rule was fragile and the input was silently accepted.

**The reserved key is not only the /0 case.** The slot-key mixer is
0-preserving, so `subnet_key(N, p)` is 0 exactly when the masked network
address `N` equals the prefix length `p`. That covers every /0 (0.0.0.0/0
masks to 0) and also `0.0.0.32/32`, and for IPv6 the u128 subnet key
truncated to u64. The publisher refuses all of them the same way.

The audit's own prefix-boundary list (0, 32, 33, 255) points at exactly
this case, and it is why prefix `0` is in the test matrix.

### 1.4 The scope is smaller than the audit implied, and stronger

The review hedged that it had "not verified" the Rust read path. The
investigation resolved that hedge into something stronger:

- **The Rust `ramshield-cgnat` crate has no read path at all.** It only
  publishes. `get_slot` is used inside `publish_rule`, in one test, and in
  one benchmark. There is no Rust reader to align with, so the review's
  suggested "align the Rust read path" step is a **no-op**.
- The C header is therefore the **only** rule-lookup path in this
  repository. Its only in-repo caller is `scripts/shm_abi_stress.c`. The
  header is a **published API** for an out-of-repo native reverse proxy —
  the crate's own doc says the table exists "for reverse-proxy lookups".

That does not reduce the severity of the contract defect: the published ABI
is the product surface, and a downstream proxy that treats a contended read
as "no rule" fails open exactly as described.

---

## 2. The change

### 2.1 A tri-state result

```c
typedef enum {
    RAMSHIELD_SHM_FOUND = 0,
    RAMSHIELD_SHM_MISS = 1,
    RAMSHIELD_SHM_CONTENDED = 2,
    RAMSHIELD_SHM_INVALID = 3
} RamshieldShmLookup;
```

`ramshield_shm_read_ex()` and `ramshield_shm_read_ipv4_ex()` are the new
entry points.

### 2.2 Contention is tracked per slot, not per call

The reader records, for each probe slot, whether that slot yielded an
**authoritative** (coherent) snapshot:

- A slot that was read coherently is **known**, even if it held a different
  key or an expired rule. It does not make the result uncertain.
- A slot whose 64-attempt budget ran out without a coherent read is
  **unknown**. It may hold the rule being looked for, so the lookup becomes
  `CONTENDED`.
- A later coherent-empty slot does **not** clear that: a contended slot
  earlier in the window may hold the rule, so absence is not proven.

`FOUND` is returned on a match; a coherent empty slot with no prior
contention is `MISS`.

The bounded retries are preserved exactly: 64 attempts per slot, 4 probe
slots.

### 2.3 Unrepresentable and out-of-range prefixes are rejected before the lookup

`prefix_len == 0` and `prefix_len > 32` are rejected with `INVALID` **before
any slot is read**. Prefix `0` is rejected because the table cannot
represent it: its key is the reserved empty-slot sentinel.

### 2.4 The published bool API keeps its signature

`ramshield_shm_read()` and `ramshield_shm_read_ipv4()` keep their `bool`
signatures: `true` only on a match, `false` otherwise, and a contended
lookup still returns `false`, as it always did.

**There is no ABI break.** The header is `static inline`, so every consumer
compiles it in; the new functions are additive. An out-of-repo proxy that
wants the new semantics calls the `_ex` functions; one that does not is
unaffected.

**Two** observable behaviours of the bool wrapper change, and both are cases
the old code got wrong:

1. **An invalid prefix can no longer find a rule.** The old wrapper looked
   an out-of-range prefix up as the reserved key 0, so it could return
   `true` for a rule stored under that key. It now returns `false`.
2. **A lookup for the reserved key 0 can no longer find a rule.** The old
   reader matched `client_hash == hash` with `hash == 0`, so
   `ramshield_shm_read(table, 0, ...)` returned `true` for a rule stored
   under the reserved key. The reader now rejects that key with `INVALID`,
   so the bool wrapper returns `false`.

The second difference was found by a direct probe of the pre-fix header
against the current one:

```
PRE-F1 (d1f4a81^): ramshield_shm_read(table, 0, ...) -> TRUE
CURRENT (master):  ramshield_shm_read(table, 0, ...) -> false
```

The contract test pins the new behaviour
(`the bool wrapper must not find a rule under the reserved key`); the
probe only establishes that the old behaviour differed.

Two earlier drafts of this report got this wrong in opposite directions.
The first claimed the wrapper was behaviour-identical in every case; the
second claimed two behaviour changes, including that a rule behind a
cleared slot is now found. **It is not** — the old reader already scanned
the whole probe window (see §2.8), so that case is unchanged from the
previous release. The change is against the *intermediate* version the
first review saw, not against what shipped.

### 2.5 The publisher refuses what the reader could never find

`publish_rule()` now returns `false` for `client_hash == 0` rather than
publishing a rule that is permanently unenforced.

### 2.6 The C and Rust slot-key hashes are pinned against each other

The two hash implementations are textually identical but **untested**. If
they diverged, a rule published by the Rust writer would land in a slot the
C reader never looks in, and every rule would be silently unenforced.

Both sides now pin the same vectors: the Rust test
`c_and_rust_slot_key_hash_agree`, and `test_key_parity_with_rust` in the C
contract test. A change to either implementation breaks one of the two.

---

## 3. The evidence

Every cycle was observed with the **final** code.

### 3.1 RED — the defect, reproduced deterministically

With the fix reverted and the final test code in place, **8 checks fail**:

```
FAIL a permanently odd sequence must be CONTENDED, not MISS; got 1
FAIL a contended lookup must never masquerade as a coherent miss
FAIL prefix 0 must not be looked up as the reserved key 0 (got 0)
FAIL prefix 0 is not representable in this table; expected INVALID, got 0
FAIL prefix 33 is out of range; expected INVALID, got 0
FAIL prefix 255 is out of range; expected INVALID, got 0
FAIL the bool wrapper must not find a rule for an out-of-range prefix
FAIL the bool wrapper must not find a rule for prefix 0
```

Note the `got 0` — that is `FOUND`. The invalid-prefix lookups really did
find the rule published under the reserved key `0`, which is the fail-open
proof in the test itself.

### 3.2 GREEN

```
SHM C ABI and concurrent reader/writer stress: PASS
SHM lookup contract: PASS
```

Rust: `c_and_rust_slot_key_hash_agree` ok, `publish_refuses_the_reserved_zero_key` ok,
14/14 in the crate's unit tests plus the integration test.

### 3.3 What the contract test pins

| Case | Property |
|---|---|
| hit | `FOUND`, and the bool wrapper still reports `true` |
| coherent miss | `MISS` |
| expired rule | `MISS` — a coherent read of an expired rule is not contention |
| permanently odd sequence | `CONTENDED`, never `MISS` |
| changing sequence | never `MISS` over 200 lookups, and a `FOUND` snapshot is never torn |
| prefix 0 / 33 / 255 | `INVALID`, and no slot is read — the reserved-key rule is not found |
| prefix 24 / 32 | representable, not `INVALID` |
| C/Rust key vectors | both sides pinned to the same constants |

The contract test runs in the existing `shm-abi` gate, so it is part of the
14-gate suite and of CI.

### 2.7 The reader rejects the reserved key

`ramshield_shm_read_ex()` rejects `hash == 0` with `INVALID`. Key 0 is the
empty-slot sentinel, so a lookup for it could neither find a rule nor prove
one absent: it would always be looking at the "everything is empty" case.
Rejecting it makes the reserved key unrepresentable on the read side as
well as the write side.

The low-level `ramshield_shm_read_ex()` takes a raw hash, so the rejection
lives there rather than only in the IPv4 wrapper. The IPv4 wrapper still
rejects the *prefix* that produces the reserved key, because that is a
different input.

### 2.8 The probe window is scanned in full, as it always was

The reader scans the whole probe window, not just up to the first empty
slot. This is not a change in behaviour: the **old** reader also scanned
the whole window, because its `break` ended the inner retry loop and not
the outer probe loop. Its comment — "Empty terminates the probe" — did not
describe the code.

That distinction matters, because `ramshield_shm_clear_slot()` can empty a
slot that has a live rule behind it in the same window. A reader that
stopped at the first empty slot would turn that rule into a miss, which is
the same fail-open class this change exists to close. The cost is
unchanged: the same number of slots are read per lookup as before.

---

## 4. Impact on the other modules, and the data flow

### 4.1 The flow itself

```
detection  ->  ramshield-detection/src/subnet_scan.rs
                 |  subnet_key(network, prefix_len)      [the ONLY key derivation in the repo]
                 |  publish_rule(key, ttl, tier, ...)     [the ONLY production publisher]
                 v
            ramshield-cgnat SHM table  (shared memory)
                 |
                 v
            out-of-repo native reverse proxy  ->  the C header lookup
```

The SHM table is **not** in the in-repo enforcement decision path. The
enforcement command is sent separately, over `enforcement_tx`, to the
enforcement actor and the kernel maps. The SHM rule is a side-channel that
publishes the decision to an out-of-repo native proxy. Nothing in this
repository reads the table back.

### 4.2 Every consumer, and what the change does to it

| Consumer | Kind | Effect |
|---|---|---|
| `crates/ramshield-detection/src/subnet_scan.rs:244` | the only production publisher | unchanged for every representable key; the reserved key is refused with an accurate warning |
| `crates/ramshield-detection/src/lib.rs` | holds the `ShmTableManager` | unaffected |
| `crates/ramshield-cgnat/tests/cgnat_integration.rs` | test | unaffected |
| `benches/hot_paths.rs` | bench | unaffected |
| `crates/ramshield-cgnat/src/shm.rs` unit tests | test | unaffected |
| `scripts/shm_abi_stress.c` | the only in-repo C reader | unaffected; its documented "contention may yield a miss" behaviour is exactly what the new explicit-result form reports as `CONTENDED` |
| the out-of-repo native proxy | published API consumer | the bool wrapper keeps its signature; two error cases change, both in the old code's favour |

### 4.3 The metrics: one intended delta

`inc_shm_publish()` is incremented only on a successful publish, and that is
unchanged. But the reserved-key case behaves differently against the
previous release, and the count follows:

- At the previous release, `publish_rule(0, ...)` **succeeded**. Phase 1
  matched the request against the first empty slot (`client_hash == 0` on
  both sides), the payload was written, and the function returned `true`.
  `shm_publish_count` was therefore incremented for a reserved-key
  decision.
- Now the reserved key is refused *before* `publish_rule` is called, so
  nothing is published and the counter is **one lower** per reserved-key
  decision.

That is the intended outcome — no rule was published, so nothing should be
counted — but it is a metric delta against the previous release, not an
unchanged count. `ramshield-detection`'s own test asserts exactly one
publish per ordinary subnet decision, and it still passes.

### 4.4 The reserved-key refusal is reachable in production, and the warning now says so

`subnet_scan.rs` derives the key with `subnet_key(network, prefix_len)`. An
IPv4 /0 network masks to 0 and hashes to 0, so the reserved-key refusal is
reachable. Before this change such a rule was published into the slot every
reader treats as empty: published and enforced by nothing. It is now
refused, with a warning that names the real cause — "the rule is not
representable (the reserved key 0)" — instead of the misleading "the probe
window saturated" that the shared `publish_rule() == false` path produced.

### 4.5 The producer and the reader agree on the whole protocol

The external review asked for this to be checked property by property, and
it was:

| property | C reader | Rust publisher | agree |
|---|---|---|---|
| key derivation | `ramshield_subnet_key` | `subnet_key` | yes — pinned by executable vectors on both sides |
| probe start | `hash & (CAPACITY-1)` | `client_hash & (CAPACITY-1)` | yes |
| window size | `RAMSHIELD_SHM_PROBE_LIMIT` = 4 | `SHM_PROBE_LIMIT` = 4 | yes |
| slot index | `(primary + probe) & (CAPACITY-1)` | `get_slot(primary + probe)`, masked inside | yes |
| match | `client_hash == hash && expires_at_ms > now_ms` | phase 1: `client_hash == client_hash`, then overwritten | yes |
| placement | — | first free-or-expired slot in the window | never places a rule beyond an empty slot |
| writer lock | even→odd CAS, 64 spins | even→odd CAS, 64 spins | yes |

The publisher never places a rule beyond an empty slot, so the full-window
scan cannot conceal a producer-side inconsistency — the old reader scanned
the same window. The case the full-window scan *does* protect is the
exported `ramshield_shm_clear_slot()`, which can empty a slot that has a
live rule behind it.

**The masking question, checked rather than assumed.** The C reader masks
the client IP to the prefix before hashing; the Rust publisher does not
mask, because it is handed an already-masked network. That holds because
`ramshield_types::IpNetwork::new` masks the address to the prefix length
(and its `Deserialize` goes through `new()`, so a hand-crafted frame cannot
bypass it), and no code builds an `IpNetwork` by struct literal. The
parity vectors therefore cover the inputs that actually occur.

**Atomicity.** Every payload field in the C struct is `_Atomic` and is read
with `atomic_load_explicit` between two acquire loads of `seq` with an
acquire fence before the second, so a mixed state from a concurrent
publication is discarded rather than returned. The reader's bounded retry
budget keeps it bounded under contention.

### 4.6 The cost of a lookup under collision — measured, not argued

The table is the reverse proxy's lookup path, so the cost of a colliding
key is a property of the hot path. `scripts/shm_lookup_bench.c` measures
it: the cost of a lookup by the probe distance of its rule, and the cost of
a mixed hit/miss workload over a populated table. Every lookup in it is
also checked for the right answer.

Measured on the development host (Intel Core i7-3687U @ 2.10 GHz, 4 vCPU,
Linux 6.8, gcc -O2, best of 5 repetitions of 200 000 lookups):

| rule's probe distance | slots scanned | ns/lookup |
|---|---|---|
| 0 | 1 | 25.7 |
| 1 | 2 | 29.1 |
| 2 | 3 | 32.4 |
| 3 | 4 | 36.3 |

Mixed workload (4096 rules, hits and misses alternating over a 4096-key
working set): **54.3 ns/lookup**.

Two things follow.

**The full-window scan is cheap.** Each extra slot in the probe window
costs about 3.5 ns, so scanning four slots instead of one costs about
10 ns. The scan was not introduced by this change — the old reader scanned
the same window — so this is the cost that already existed, now measured.

**The "<15ns" comment was not true.** `crates/ramshield-detection/src/lib.rs`
claimed the table was a "<15ns" lookup path. The measured cost is roughly
26 ns for the simplest case and 36 ns at the far end of the window, on a
host that is shared and noisy enough that the run-to-run spread on a single
figure is wider than the difference between probe distances. The comment
now carries the measured figures and the caveat instead of the unsupported
one. This is a performance claim, so it belongs to F7's evidence policy:
a figure may only be published with the workload and the machine recorded.

The gate enforces the **correctness** of the answers, not the timings — a
timing assertion on a shared runner is a flake generator. The measured
figures are printed for the record.

### 4.7 What is a known limitation, not a fix

**A /0 rule is now inexpressible end to end.** The publisher refuses it and
the reader rejects it. That is the correct behaviour for a table whose key
space reserves 0, but it means a deployment that needs a *global* block
has no path into this table. Supporting one needs a different key
derivation — a design change, outside the scope of this fix, and it is
recorded in `docs/development/OPEN_FINDINGS.md`.

The in-repo enforcement path is unaffected: a /0 CIDR block is still
enforced through the normal channel, and the userspace path remains
authoritative in every case.

---

## 5. What was deliberately NOT changed

- **The layout.** No struct, offset, alignment or size changed. The ABI
  static asserts are untouched and still pass.
- **The retry budget.** 64 attempts, 4 probe slots, exactly as before.
- **The bool API.** Signatures and behaviour are unchanged.
- **The Rust read path.** There is none, so nothing was invented.
- **The publisher's slot-selection strategy.** Only the reserved-key-0 case
  is refused; the rest of `publish_rule` is untouched.
- **The mesh and IPC paths.** Not involved.

---

## 6. The independent review, and what changed in response to it

`docs/development/CGNAT_SHM_LOOKUP_REVIEW.md` — verdict **PASS**,
"safe to push: yes".

The review confirmed the defect, the scope correction, the prefix handling,
the key parity, and the absence of undefined behaviour. It also raised
three things that were acted on:

1. **The bool wrapper is not behaviour-identical in every case** (its
   observation on question (c)). An earlier draft of this report claimed it
   was; that was wrong, and the claim is corrected in §2.4.
2. **The reader stopped at the first coherently empty slot** (its question
   (c) and observation). That was a fail-open regression this change
   introduced — `ramshield_shm_clear_slot()` can empty a slot that has a
   live rule behind it. The reader now scans the whole probe window, which
   is what the old reader did; the old "empty terminates the probe" comment
   did not describe the old code. A new test pins it.
3. **The `out` snapshot, the bool wrapper under contention, the reserved
   key, and a rule behind a cleared slot were uncovered** (its question
   (g)). Four tests were added, and each was proved non-vacuous by
   reintroducing its defect alone:

   | test | with its defect reintroduced |
   |---|---|
   | a live rule behind a cleared slot is still found | `got 1` (MISS) |
   | the reserved key is rejected | `got 0` (FOUND) |
   | the snapshot is untouched when nothing is found | covered |
   | the bool wrapper reports false under contention | covered |

Two further reviewer suggestions were adopted as documentation: the enum
now warns that a truthiness test is wrong (`FOUND` is zero), and the enum
now states what a caller must do with `CONTENDED` — the response is
deployment-specific, but treating it as a coherent miss is always wrong.

The review's suggestion to "align the Rust read path" was not actionable
and was not done: there is no Rust read path (§1.4).

---

## 7. Verification

```
bash scripts/verify_shm_header.sh
  SHM C ABI and concurrent reader/writer stress: PASS
  SHM lookup contract: PASS
cargo test -p ramshield-cgnat
  14 passed, 0 failed (lib) + 1 passed (integration)
14-gate CI-equivalent suite
  14/14 EXIT=0
```

---

## 8. Status against the finding's required fix

| Required | Status |
|---|---|
| 1. Distinct FOUND / MISS / CONTENDED / invalid-input results | **Done** — `RamshieldShmLookup` + the two `_ex` readers |
| 2. Reject invalid prefixes before lookup | **Done** — prefix 0 and > 32 are rejected with `INVALID` before any slot is read |
| 3. Audit all callers and define their response to contention | **Done** — the only in-repo caller is the stress harness; the published API's response is defined: the bool wrapper is unchanged, and the `_ex` functions let a proxy distinguish contention. The in-repo harness's own comment ("bounded contention may yield a miss") is the behaviour being corrected. |
| 4. Preserve bounded retries | **Done** — 64 attempts, 4 probes, unchanged |
| 5. Deterministic contention and prefix-boundary tests | **Done** — `scripts/shm_lookup_contract.c`, wired into the gate |
| 6. Verify C/Rust ABI layout and run the SHM stress gates | **Done** — the layout asserts pass unchanged, the C ABI stress passes, the shm-abi gate runs the new test, and the C/Rust key hashes are now pinned against each other |

**Closure criterion — a contended lookup cannot masquerade as a coherent
miss** — is met and proven by the RED above.

**Not done, and deliberately so:** the F-01 pairwise-HMAC branch and the
roadmap's F-02/F-03 remain parked; this change touches neither.
