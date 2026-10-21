# CGNAT SHM LOOKUP — DELTA REVIEW (post first-review changes)

**Scope:** the delta applied to `crates/ramshield-cgnat/include/ramshield_shm.h`,
`crates/ramshield-cgnat/src/shm.rs`, `crates/ramshield-detection/src/subnet_scan.rs`,
`scripts/shm_lookup_contract.c`, plus the changed docs
(`crates/ramshield-cgnat/README.md`, `docs/development/CGNAT_SHM_LOOKUP_FIXES.md`).
Base: `191a7d4` (= `origin/master`). Old file read via
`git show HEAD:crates/ramshield-cgnat/include/ramshield_shm.h`.

VERDICT: PASS
SAFE TO PUSH: no

REQUIRED CHANGES

1. `docs/development/CGNAT_SHM_LOOKUP_FIXES.md:137-139` (§2.4 item 2),
   `crates/ramshield-cgnat/README.md:45-53` (Release note), and
   `crates/ramshield-cgnat/include/ramshield_shm.h:212-219` (wrapper comment)
   state that a rule sitting behind a slot cleared by
   `ramshield_shm_clear_slot()` **is now found**, and that this is an
   observable behaviour change from the previous release. It is not: the
   reader at `HEAD` already scanned the whole probe window — its
   `if (client_hash == 0) break;` ended the **inner retry loop**, not the
   outer probe loop (`git show HEAD:.../ramshield_shm.h`, the `break` sits
   inside `for (attempt = 0; attempt < 64; ++attempt)`; the outer
   `for (probe ...)` continues to the next slot). Only **one** bool-wrapper
   behaviour differs from the previous release: the invalid-prefix case
   (old: prefix 0 or >32 was looked up as key 0 and could return `true`;
   new: `false`/INVALID). The "cleared slot" case is a difference against
   the *intermediate* version the first review saw, not against `HEAD` —
   the delta itself says so in `CGNAT_SHM_LOOKUP_FIXES.md:223-235` (§2.8),
   which directly contradicts §2.4. Fix: keep one behaviour difference in
   §2.4, in the README release note and in the header comment, and state
   that the full-window scan is preserved (not new) behaviour.
   The header comment must not tell an out-of-repo consumer that its
   previously-shipped reader had a cleared-slot miss bug; it did not.

   The same list is incomplete in the other direction: at `HEAD`,
   `ramshield_shm_read(table, 0, ...)` and `ramshield_shm_read_ipv4(...,
   0, ...)` could return **true** for a rule published under key 0; both
   now return false. That is a third observable bool-wrapper difference
   against the previous release and it belongs in the enumeration. It
   shares item 1's root cause but is not covered by the "invalid prefix"
   wording, because the low-level route takes a raw hash.

2. `docs/development/CGNAT_SHM_LOOKUP_FIXES.md:272-277` (§4.3) claims the
   `shm_publish` counter is unchanged because "the publish failed". Against
   the previous release it is not unchanged: at `HEAD`,
   `publish_rule(0, ...)` **succeeded** (phase 1 matched the first
   zero-hash slot, the payload was written with `client_hash = 0` and a
   live expiry, and the function returned `true`), so
   `crates/ramshield-detection/src/subnet_scan.rs:251` incremented
   `shm_publish_count` for a /0 subnet. With this change the reserved key
   is refused before `publish_rule` is called
   (`crates/ramshield-detection/src/subnet_scan.rs:232-243`), so the
   counter is one lower per reserved-key decision. That is the intended
   outcome (no rule was published, so nothing should be counted), but it
   is a metric delta against `HEAD` and the report must say so instead of
   claiming "unchanged".

3. `docs/development/CGNAT_SHM_LOOKUP_FIXES.md:46-56` (§1.3),
   `docs/development/OPEN_FINDINGS.md` (F1 / F1a text),
   `crates/ramshield-cgnat/README.md:66-74` (Prefix lengths) and the header
   comment at `crates/ramshield-cgnat/include/ramshield_shm.h:29-35` /
   `:131-134` claim that a /0 rule "could be published and never found:
   enforced by nothing". Against `HEAD` that is not what happened: the
   old reader matched `client_hash == hash` with `hash == 0`, so a rule
   published under key 0 (the old publisher allowed it) **was** found by
   a prefix-0 lookup. The report's own RED evidence says so: "prefix 0
   must not be looked up as the reserved key 0 (got 0)" — `got 0` is
   `RAMSHIELD_SHM_FOUND`, i.e. the old code *found* the key-0 rule. The
   real `HEAD` hazards were (i) the rule is indistinguishable from an
   empty slot, so the publisher treats its slot as free and another rule
   can overwrite it, and (ii) a prefix-0 lookup was routed to key 0 with
   no error. The new code removes both; the prose must not claim a
   behaviour that `HEAD` did not have. Same wording applies to the
   "a lookup for key 0 could neither find a rule nor prove one absent"
   justification: true only *after* the publisher starts refusing key 0,
   not against `HEAD`.

No C/Rust code change is required for any of the three items; all three
are corrections to the shipped prose. The code in the delta is correct as
far as I could verify by reading (see the table). Note that item 3 also
softens a claim the delta makes about the *old* behaviour, not about the
new behaviour, which is right.

| # | Question | Answer | One-line reason | Evidence |
|---|---|---|---|---|
| a | Full-window probe scan correct? Behaviour change vs old? | YES, correct; **no** behaviour change vs `HEAD` | the old `break` ended the inner retry loop, so `HEAD` already scanned all 4 slots; the new code scans the same 4 slots, so per-lookup cost and the find-set are identical | `include/ramshield_shm.h:141-205` (new) vs `git show HEAD:.../ramshield_shm.h` lines 93-138 (old `break` at old line 48 inside the attempt loop) |
| b | `hash == 0` rejection correct/complete? Any remaining route? | Correct and complete for the reserved key | the low-level reader rejects key 0 before touching a slot; the IPv4 wrapper rejects prefix 0 and >32 before any lookup; the helper's 0 return is now unreachable-as-valid-key | `include/ramshield_shm.h:135-137`, `:257-259`, `:28-37` |
| c | Do the two documented bool-wrapper behaviour changes hold? | #1 YES, #2 **NO** | invalid-prefix case really changed (old could find a key-0 rule); the cleared-slot case was already the old behaviour, and §2.4 contradicts §2.8 of the same report | `git show HEAD:.../ramshield_shm.h` old `ramshield_shm_read_ipv4` + old `publish_rule` phase 1; `CGNAT_SHM_LOOKUP_FIXES.md:130-142` vs `:223-235`; `README.md:45-53`; `include/ramshield_shm.h:212-219` |
| d | `subnet_scan.rs` change correct/minimal? Metrics? Userspace path? Spam? | Correct and minimal; small metric delta; userspace enforcement unaffected; warn is tick-rate bounded | the reserved key is detected before `publish_rule`, the warning names the real cause, and the enforcement command is sent on a separate path; the counter is one lower per reserved-key decision than at `HEAD` | `crates/ramshield-detection/src/subnet_scan.rs:225-256`; enforcement send at `:186-206`; `crates/ramshield-cgnat/src/shm.rs:252-258` |
| e | Four new tests correct, non-vacuous, no UB? | Yes, and non-vacuous by construction | each pins a property the code must have: full-window scan, `out` untouched on a miss, `false` under contention, reserved key rejected; no UB (all accesses are `_Atomic` loads/stores on a heap table) | `scripts/shm_lookup_contract.c:270-286` (cleared slot), `:291-308` (snapshot), `:313-326` (bool under contention), `:330-342` (reserved key) |
| f | Batch minimal / conventions / UB / memory-safety / concurrency? | Yes; no defect found | additions are additive (`_ex` functions beside the old ones), the retry budget is unchanged (64 attempts x 4 slots), the seqlock protocol and memory orders are untouched, layout asserts unchanged | `include/ramshield_shm.h:59-71` (asserts), `:149-165` (seq protocol), `:225-232` (wrapper) |
| g | Changed docs accurate? Old reader full-window claim? Per-lookup cost? §2.4 wrapper differences? | Full-window claim and per-lookup cost claim: accurate. §2.4 / README / header wrapper-difference claim: **inaccurate** | `HEAD` did scan the full window and the cost is unchanged, but §2.4, the README release note and the header comment present a second behaviour difference that does not exist against `HEAD` | `CGNAT_SHM_LOOKUP_FIXES.md:223-235` (correct) vs `:130-142` (wrong); `README.md:45-53`; `include/ramshield_shm.h:212-219` |

OPTIONAL OBSERVATIONS

- **Reserved key is reachable beyond /0.** `subnet_key(N, p)` is 0 exactly
  when `N == p` (the mixer is 0-preserving), so besides `0.0.0.0/0`
  (`subnet_key(0,0) == 0`) the host prefix `0.0.0.32/32` also hashes to 0
  (`subnet_key(32,32) == 0`). For canonical CIDRs (network a multiple of
  2^(32-p)) the only IPv4 hits are `0.0.0.0/0` and `0.0.0.32/32`. I
  verified the arithmetic; the C and Rust
  sides agree, so both refuse it consistently, and the reader returns
  INVALID. The prose, however, only ever mentions /0
  (`include/ramshield_shm.h:29-35`, `CGNAT_SHM_LOOKUP_FIXES.md:46-56`).
  UNVERIFIED: whether `0.0.0.32/32` can actually reach the publisher
  through the subnet table.
- **Rust masks, C masks, but parity is only pinned for masked inputs.**
  The Rust production key is `subnet_key(cidr.addr, cidr.prefix_len)`
  (`subnet_scan.rs:225-230`) with no masking, while the C reader masks
  the client IP (`include/ramshield_shm.h:38-39`). The parity vectors
  only cover already-masked networks, so a non-masked CIDR in the subnet
  table would silently diverge. Pre-existing, outside this delta.
- **The Rust /0 parity vector is not a reader-reachable vector.**
  `subnet_key(0xC0000200, 0) == 0xab616229` (C test:251, Rust test)
  corresponds to an *unmasked* /0 network; the C reader always masks /0
  to network 0, i.e. key 0. Harmless, but the vector does not pin the
  production /0 correspondence.
- **`ramshield_ipv4_subnet_key()` change for prefix 0 is documentation
  only.** At `HEAD` the helper already returned 0 for prefix 0
  (`mask = 0` -> `subnet_key(0,0) == 0`); the new early return adds no
  behaviour, only an explicit contract. The `prefix_len == 0 ? 0 :` arm
  at `include/ramshield_shm.h:38` is now dead.
- **IPv6 key path can also yield 0.** `subnet_scan.rs:229` uses
  `sk as u64` for IPv6, which is 0 for at least one subnet key, so the
  "reserved key 0" warning can be emitted for a v6 subnet; the message
  still names the real cause (the key), so it is not misleading, but the
  comment at `:233-237` names only the IPv4 /0 case.
- **Reachability of the reserved-key case in production is UNVERIFIED.**
  The refusal triggers whenever the derived key is 0 (IPv4: `0.0.0.0/0`
  and `0.0.0.32/32`; IPv6: any `sk` that is 0). Whether a /0 CIDR can
  enter the subnet table at all, and therefore how often the new warning
  fires, depends on the detection configuration and was not traced here.
- **Warning volume.** The reserved-key warning is `warn!` per scan tick
  (~2/s per hot subnet) — bounded, but a /0 present in the subnet table
  would repeat indefinitely. The neighbouring per-tick event was
  deliberately demoted to `debug!` for the same reason
  (`subnet_scan.rs:166-174`).
- **`scripts/shm_lookup_contract.c` changed during this review.** My read
  was of a 374-line version; a later grep showed ~380 lines with an
  additional colliding-rule case (`HASH_COLLIDE_A/B`). That addition is
  UNVERIFIED by me. The four tests the delta names were read and reviewed
  as quoted above.
- **Non-vacuity of the four new tests was not re-proved here** (the
  report claims each was proved by reintroducing its defect); I checked
  by inspection that each assertion fails against the corresponding
  defect, but I ran nothing (build/test execution was out of scope for
  this review).
- `test_snapshot_is_untouched_when_no_rule_is_found` asserts a property
  (`out` is not written on a miss) that the header does not document as
  a contract, only as an implementation property. Harmless, but it is
  stronger than the published contract; consider stating it in the
  header.

REVIEWED IN 13 TOOL CALLS (9 assistant turns; this file was written on turn 8)
