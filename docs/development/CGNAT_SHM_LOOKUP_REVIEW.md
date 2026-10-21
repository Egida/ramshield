# CGNAT SHM tri-state lookup — independent review (F1)

VERDICT: PASS
SAFE TO PUSH: yes

Reviewed tree (read-only): branch `fix/master-p0-p1-fixes`, HEAD `191a7d4`.
Files reviewed (md5 at review time):
- `crates/ramshield-cgnat/include/ramshield_shm.h` `34fdfac8a9072877959453fbe7df9c60`
- `crates/ramshield-cgnat/src/shm.rs` `ef3eb3fa3c5f538cf1de61a4f026ed39`
- `scripts/shm_lookup_contract.c` `c3353fc21abb0393506aef4b4fe3beb2`
- `scripts/verify_shm_header.sh` `62db00f3071a5ed44c4c3af7278fcd75`

No mutating command was run (no `cargo fmt` without `--check`, no `cargo test`, no
`cargo clippy`, no git write, no `scripts/verify_shm_header.sh`).

| # | Question | Answer | Reason (one line) | Evidence |
|---|---|---|---|---|
| a | Contention logic correct? Can a contended lookup still be a coherent MISS? Can an absent key be reported CONTENDED? Is the "coherent empty slot" reasoning sound? | Tri-state logic **correct**; MISS is unreachable while a probed slot is contended; **false-positive CONTENDED is possible**; the empty-slot reasoning holds for the in-repo writer but is now load-bearing | The `contended` flag is sticky and is only cleared by a `FOUND`; an incoherent slot sets it and the next coherent-empty slot returns `CONTENDED`, never `MISS` | `crates/ramshield-cgnat/include/ramshield_shm.h:117` (flag), `:145` (`coherent=true`), `:167-168` (sticky return), `:177-179` (budget-exhausted ⇒ contended), `:181` (final) |
| b | Prefix rejection correct and complete? Is rejecting /0 right? Any other prefix/input mishandled? | **Correct and complete for 0 and >32**; rejecting /0 is right for this table (key 0 is the reserved sentinel) but it removes the ability to express a block-all rule | `ramshield_ipv4_subnet_key(ip,0)` masks to network 0 ⇒ `ramshield_subnet_key(0,0) == 0` = the empty sentinel, so /0 is unrepresentable; `prefix_len` is `uint8_t`, so 0..255 is the full input domain and 33..255 is rejected | header `:26-33` (mask, `>32 ⇒ 0`), `:224-226` (reject 0 and >32 before any slot read), `:28-30` (`ramshield_ipv4_subnet_key` still returns 0 for >32) |
| c | Is the bool wrapper genuinely behaviour-identical for every reachable input? Can an out-of-repo consumer observe a difference? | **No — two observable differences** (not a defect by itself, but the header's "exactly as it always was" wording is inaccurate) | (i) a coherent **empty** slot now terminates the probe, while the old code `break`ed only the inner retry loop and kept probing the remaining probe slots; (ii) `ramshield_shm_read_ipv4` now rejects prefix 0 and >32 instead of looking up key 0 (which could return **true** for a rule published under the reserved key) | old code `break` at `git show HEAD:…/ramshield_shm.h` ("if (client_hash == 0) break;") vs new `return contended ? CONTENDED : MISS` at header `:167-168`; wrapper `:192-199`, `:235-244` |
| d | Is the publish-side guard (refuse key 0) correct and sufficient? Other ways a rule is published that a reader never finds? | Guard **correct and sufficient in-repo**; two residual paths | Key 0 is the empty sentinel on both sides (Rust `client_hash == 0` ⇒ free slot; C reader treats it as empty), so refusing it is right; the Rust writer never zeroes a slot (it overwrites expired ones), so no in-repo writer creates a "hole"; but (i) a C-side `ramshield_shm_clear_slot()` can zero a slot that has a live rule **behind** it in the same probe window, and (ii) `subnet_scan.rs` reports *any* `publish_rule()==false` as "probe window saturated", which is now also the zero-key refusal path | `crates/ramshield-cgnat/src/shm.rs:253-258` (refusal), `:264-282` (phases 1–2, no zeroing), `:315-320` (claimable), header `:267` (`clear_slot` writes `client_hash = 0`), `crates/ramshield-detection/src/subnet_scan.rs:232-247` (misleading warning) |
| e | Are the C and Rust key vectors correct? Does the parity test catch divergence in either direction? Is the masking C `ramshield_ipv4_subnet_key` consistent with non-masking Rust `subnet_key`? | **Vectors correct; parity test catches divergence in either direction; the masking difference is benign but convention-fragile** | `ramshield_ipv4_subnet_key(ip,p) == ramshield_subnet_key(ip & mask, p) == Rust subnet_key(masked_network, p)`, so C and Rust agree iff the Rust publisher is handed an already-masked network (`IpNetwork::new` normalizes); the two test files pin the *same* constants, so a change to either implementation breaks the other side's test | `scripts/shm_lookup_contract.c:236-245`, `crates/ramshield-cgnat/src/shm.rs:648-656`; Rust `subnet_key` at `crates/ramshield-cgnat/src/shm.rs:18-27`; C at header `:13-33`; publisher call site `crates/ramshield-detection/src/subnet_scan.rs:225-228` |
| f | Correctness / portability / UB / memory safety / concurrency defects in the new C code or test? C11 atomics correct? Is the churn thread worse than a real writer? | **No defect found in the new code**; two portability/ergonomics notes | The seqlock read pattern (relaxed payload loads, acquire fence, second acquire `seq` load) is the standard idiom and is unchanged by this diff; all struct members are `_Atomic`, so no torn read and no C11 data-race UB; `aligned_alloc` size is a multiple of the 64-byte alignment, `atomic_init` used correctly; the churn thread only stores `seq` (atomic) so it introduces no race — it is *not* a compliant writer protocol (a real writer goes even→odd→even), so it stresses a state a compliant writer produces only transiently, and it still cannot produce a false `MISS` because the slot never reads coherently empty | header `:127-143` (read loop), `:36-50` (all-atomic entry), `scripts/shm_lookup_contract.c:135-146` (churner), `:250-256` (`aligned_alloc`/alignment) |
| g | Is the test set adequate and non-vacuous? What is NOT covered that matters? | **Adequate for the fix's own claims, non-vacuous; five coverage gaps** | Each new assertion has a demonstrated RED (odd-seq ⇒ `CONTENDED`, prefix 33 ⇒ `INVALID` and the key-0 trap rule is really `FOUND` under the old code) — the tests fail on the old code, so they are not vacuous; uncovered: probe-collision paths, expired-slot-0 + live-rule-at-1, bool wrapper under contention, cross-language publish→C-read roundtrip, `out` untouched on non-`FOUND` | `scripts/shm_lookup_contract.c:66-107` (hit/miss/expired), `:114-130` (odd seq), `:148-189` (churn), `:197-227` (prefix boundaries), `:234-246` (parity) |
| h | Minimal and consistent with repo conventions? Does the header's documentation now match its behaviour? | **Minimal and consistent; two documentation statements are now inaccurate** | The diff adds one enum + two functions + two thin wrappers and touches nothing else; `cargo fmt --check -p ramshield-cgnat` is clean; the C file follows `scripts/shm_abi_stress.c` conventions and is wired into the existing gate — but the wrapper comment "exactly as it always was" and the `ramshield_ipv4_subnet_key` "reject invalid prefix" contract no longer describe actual behaviour | header `:184-191` (wrapper doc), `:28-30` (helper contract), `scripts/verify_shm_header.sh:18-25` (new gate step), `crates/ramshield-cgnat/src/shm.rs:641-670` (new Rust tests) |

## OPTIONAL OBSERVATIONS (non-blocking)

1. **The bool wrapper is not behaviour-identical to the old code** (see row c). The
   difference that matters for an out-of-repo consumer: the old reader continued
   probing the remaining probe slots after a coherent empty slot; the new reader
   returns `MISS` at the first coherent empty slot. Under the in-repo Rust writer
   this is harmless (the writer places a rule in the *first* free/expired slot and
   never zeroes a slot, so an empty slot before a live rule cannot occur). It
   becomes observable only if a writer empties a slot that has a live rule behind
   it in the same window — reachable through the exported
   `ramshield_shm_clear_slot()` (header `:248-277`), which has no in-repo caller.
   *UNVERIFIED* whether any out-of-repo native writer uses `ramshield_shm_clear_slot`
   that way. If one does, this change converts a previously-found rule into a
   `MISS` (fail-open) — the exact class of defect the change exists to close.
   Cheap mitigation: keep the sticky `contended` flag but only treat a coherent
   empty slot as authoritative if no earlier slot was cleared, or have the Rust
   publisher re-use (rather than leave) cleared slots. Not blocking.
2. **`ramshield_shm_read_ipv4` behaviour change for invalid prefixes** is
   intended (the old code could return `true` for the reserved key-0 rule), but it
   is a live ABI-visible change for out-of-repo consumers. Worth a line in the
   release notes for the published header.
3. **The wrapper comment is inaccurate**: "reported as 'not found', exactly as it
   always was" (header `:185-187`) — it *is* exactly as before for contention, but
   not for the empty-slot and invalid-prefix cases. Reword to "behaviour is
   unchanged except that a lookup which cannot be completed coherently is still
   reported as 'not found'".
4. **`ramshield_ipv4_subnet_key()` still returns 0 for `prefix_len > 32`**
   (header `:28-30`) and now also returns 0 for `prefix_len == 0` (mask 0 ⇒
   network 0). The new `ramshield_shm_read_ipv4_ex()` shields the high-level API,
   but a direct C caller of the helper can still route into the reserved key 0.
   Consider documenting the helper as "returns 0 for prefixes this table cannot
   represent (0 and >32) — do not use the result as a key".
5. **/0 (block-all) is now inexpressible end to end.** The reader returns
   `INVALID` and the Rust publisher refuses key 0. That is honest, but a proxy
   that needs a global block has no path. If that is a real requirement, it
   needs a different key derivation (e.g. reserve a sentinel other than 0, or keep
   the prefix in the key so /0 does not collide with the sentinel) — a design
   change, out of scope here.
6. **CONTENDED false positives and unspecified caller action.** Contention is
   detected per *slot*, so a writer touching an unrelated colliding slot in the
   probe window makes a lookup for an absent key return `CONTENDED`. That is
   fail-safe (it is not a miss), but the enum does not say what a proxy should do
   — retry, or apply the default mitigation? The header comment only says "this
   is NOT a miss". Recommend stating the intended caller behaviour explicitly, and
   ideally distinguishing "a slot was contended" from "the rule may be present".
7. **`RAMSHIELD_SHM_FOUND == 0`** (header `:96`) makes a naive `if (ramshield_shm_read_ex(...))`
   read as failure. For a published API this is an ergonomic trap; consider
   starting the enum at 1 or documenting "compare against `RAMSHIELD_SHM_FOUND`".
8. **`ramshield_shm_read_ex()` does not reject `hash == 0`.** The reserved-key
   ambiguity (a slot with `client_hash == 0` is simultaneously "empty" and a
   match for key 0 when its `expires_at_ms` is in the future) is still reachable
   through the low-level API. Since `read_ex` is new, rejecting `hash == 0` with
   `INVALID` would close it for free.
9. **Rust test placement/naming**: the two new Rust tests live inside
   `mod path_safety` (`crates/ramshield-cgnat/src/shm.rs:558`, tests at `:641-670`),
   whose name no longer describes its contents; and
   `publish_refuses_the_reserved_zero_key` does not remove its scratch directory
   (other tests in that module do). Cosmetic.
10. **Misleading publish-failure log**: `crates/ramshield-detection/src/subnet_scan.rs:239-247`
    logs "CGNAT SHM probe window saturated" for every `publish_rule() == false`,
    which now also covers the intentional key-0 refusal. Distinguish the two.
11. **The new contract test is not covered by the ThreadSanitizer leg.**
    `scripts/verify_shm_header.sh:27-46` builds only `scripts/shm_abi_stress.c`
    under TSan; the new file spawns a thread and would be a good TSan subject
    (it is also cheap to add).
12. **Coverage gaps that matter** (in rough priority):
    - probe-collision behaviour: a rule published at `primary+1..3` must be found;
      the "occupied slot with another key ⇒ advance probe" path
      (header `:169-170`) is untested, as is the "expired rule in slot 0 while a
      live collision sits at slot 1" case the code comments call out
      (header `:159-162`).
    - the bool wrapper under a contended slot (should be `false`, i.e. unchanged).
    - a real cross-language roundtrip: a rule published by the Rust writer into
      the mmap and read back by the C reader. Today the parity is asserted
      through duplicated hash constants only, which is a weaker guarantee than
      "the C reader finds the rule the Rust writer published".
    - that `out` is left untouched on `MISS`/`CONTENDED`/`INVALID`.
13. **The churn thread models a non-compliant writer** (it stores only even `seq`
    values and never the odd phase, and never writes payload). That is fine for
    the assertion it supports (a coherent read of that slot still yields `FOUND`
    with the published payload, so the test cannot pass vacuously), but it is not
    a faithful "writer is publishing" simulation; the compliant even→odd→even
    case is only covered by the synthetic odd-seq test.
14. **Scope discrepancy vs the brief**: the brief said the uncommitted tree
    contains only the F1 change, but the working tree also shows
    `crates/ramshield-cgnat/README.md`, `docs/development/OPEN_FINDINGS.md` and
    `docs/development/OWNER_DECISION_PACKAGE.md` modified, and
    `docs/development/CGNAT_SHM_LOOKUP_FIXES.md` untracked. Those were not
    reviewed here (out of scope). `crates/ramshield-cgnat/src/shm.rs` also changed
    by one line (a trailing blank line) between my first and last read, i.e. the
    tree was being edited concurrently during the review; the md5s above pin what
    was reviewed.

## Evidence relied upon (not re-run)

- RED with the fix reverted: 8 checks fail, including "a permanently odd sequence
  must be CONTENDED, not MISS; got 1" and "prefix 33 is out of range; expected
  INVALID, got 0" (0 = `FOUND`, i.e. the invalid-prefix lookup really did find
  the reserved key-0 rule).
- GREEN with the fix: "SHM lookup contract: PASS", C ABI stress PASS, Rust crate
  14/14 plus the integration test.
- `cargo fmt --check -p ramshield-cgnat` → clean (exit 0) on the reviewed tree.

REVIEWED IN 11 TOOL CALLS
