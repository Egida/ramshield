/* Deterministic contract tests for the CGNAT shared-memory lookup API.
 *
 * The concurrency stress harness (scripts/shm_abi_stress.c) proves that a
 * reader never observes a torn rule. It cannot prove the parts of the
 * contract that are about the RESULT of a lookup:
 *
 *   - a lookup that could not be completed coherently must be reported as
 *     CONTENDED, not as a coherent miss;
 *   - a prefix length that the table cannot represent must be rejected
 *     before any slot is read, not routed into the table as key 0;
 *   - the C slot-key hash must agree with the Rust one, or every published
 *     rule becomes invisible to the proxy that is supposed to enforce it.
 *
 * Run by scripts/verify_shm_header.sh.
 */
#include "crates/ramshield-cgnat/include/ramshield_shm.h"

#include <inttypes.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int failures;

#define CHECK(cond, ...)                                                      \
    do {                                                                      \
        if (!(cond)) {                                                        \
            fprintf(stderr, "FAIL %s:%d: ", __FILE__, __LINE__);              \
            fprintf(stderr, __VA_ARGS__);                                     \
            fputc('\n', stderr);                                              \
            failures++;                                                       \
        }                                                                     \
    } while (0)

static RamshieldShmRuleEntry *table;

/* Hashes whose primary slots are all distinct, so the probe windows of the
 * different cases do not overlap. */
#define HASH_HIT  UINT64_C(0x1111)
#define HASH_MISS UINT64_C(0x2222)
#define HASH_BUSY UINT64_C(0x3333)
#define HASH_ZERO UINT64_C(0)
/* Two keys whose PRIMARY SLOT is the same: they differ only above the
 * capacity's index bits, so the second one is placed in the next slot of
 * the probe window. The reader must find each of them and never confuse
 * one for the other. */
#define HASH_COLLIDE_A UINT64_C(0x1111)
#define HASH_COLLIDE_B UINT64_C(0x41111)

static void reset_table(void)
{
    memset(table, 0, (size_t)RAMSHIELD_SHM_TABLE_CAPACITY * sizeof(*table));
}

/* Write a coherent, live rule for `hash` into its primary slot. The slot's
 * seq is left even, so the slot is stable and readable. */
static void publish(uint64_t hash, uint64_t expires_at_ms)
{
    const size_t index = (size_t)(hash & (RAMSHIELD_SHM_TABLE_CAPACITY - 1u));
    RamshieldShmRuleEntry *slot = &table[index];
    atomic_store_explicit(&slot->client_hash, hash, memory_order_relaxed);
    atomic_store_explicit(&slot->expires_at_ms, expires_at_ms, memory_order_relaxed);
    atomic_store_explicit(&slot->max_rps, 7, memory_order_relaxed);
    atomic_store_explicit(&slot->tier, 2, memory_order_relaxed);
    atomic_store_explicit(&slot->flags, RAMSHIELD_FLAG_SHARED_INFRA, memory_order_relaxed);
    atomic_store_explicit(&slot->challenge_seed_lo, UINT64_C(0xAAAA), memory_order_relaxed);
    atomic_store_explicit(&slot->challenge_seed_hi, UINT64_C(0xAAAA), memory_order_relaxed);
}

/* Publish a rule into a specific slot, not the hash's primary slot. */
static void publish_into(size_t index, uint64_t hash, uint64_t expires_at_ms)
{
    RamshieldShmRuleEntry *slot = &table[index];
    atomic_store_explicit(&slot->client_hash, hash, memory_order_relaxed);
    atomic_store_explicit(&slot->expires_at_ms, expires_at_ms, memory_order_relaxed);
    atomic_store_explicit(&slot->max_rps, 7, memory_order_relaxed);
    atomic_store_explicit(&slot->tier, 2, memory_order_relaxed);
    atomic_store_explicit(&slot->flags, RAMSHIELD_FLAG_SHARED_INFRA, memory_order_relaxed);
    atomic_store_explicit(&slot->challenge_seed_lo, UINT64_C(0xAAAA), memory_order_relaxed);
    atomic_store_explicit(&slot->challenge_seed_hi, UINT64_C(0xAAAA), memory_order_relaxed);
}

/* A coherent, live rule is found, and the compatibility wrapper agrees. */
static void test_hit(void)
{
    RamshieldShmRuleSnapshot snap;
    RamshieldShmRuleSnapshot snap2;
    reset_table();
    const uint64_t now = 1000;
    publish(HASH_HIT, now + 60000);

    const RamshieldShmLookup r = ramshield_shm_read_ex(table, HASH_HIT, now, &snap);
    CHECK(r == RAMSHIELD_SHM_FOUND, "hit: expected FOUND, got %d", (int)r);
    CHECK(snap.client_hash == HASH_HIT, "hit: wrong client_hash");
    CHECK(snap.max_rps == 7, "hit: wrong max_rps");
    CHECK(snap.tier == 2, "hit: wrong tier");
    CHECK(snap.expires_at_ms == now + 60000, "hit: wrong expiry");

    CHECK(ramshield_shm_read(table, HASH_HIT, now, &snap2),
          "hit: the bool compatibility wrapper must still report true");
}

/* A coherent lookup that finds no live rule is a MISS, not a contention. */
static void test_coherent_miss(void)
{
    RamshieldShmRuleSnapshot snap;
    reset_table();
    const uint64_t now = 1000;
    publish(HASH_HIT, now + 60000);

    const RamshieldShmLookup r = ramshield_shm_read_ex(table, HASH_MISS, now, &snap);
    CHECK(r == RAMSHIELD_SHM_MISS, "coherent miss: expected MISS, got %d", (int)r);
}

/* An expired rule is a coherent miss. */
static void test_expired_rule_is_a_miss(void)
{
    RamshieldShmRuleSnapshot snap;
    reset_table();
    const uint64_t now = 1000;
    publish(HASH_HIT, now - 1);

    const RamshieldShmLookup r = ramshield_shm_read_ex(table, HASH_HIT, now, &snap);
    CHECK(r == RAMSHIELD_SHM_MISS, "expired rule: expected MISS, got %d", (int)r);
}

/* A slot whose sequence counter is permanently odd means a writer is active
 * there. The rule may well be present, so the lookup must report contention
 * and NOT a coherent miss. Reporting a miss here is the defect this whole
 * change exists to close: a proxy that reads "no rule" stops challenging or
 * blocking a client the table does have a rule for. */
static void test_permanently_odd_sequence_is_contended(void)
{
    RamshieldShmRuleSnapshot snap;
    reset_table();
    const uint64_t now = 1000;
    publish(HASH_BUSY, now + 60000);

    const size_t index = (size_t)(HASH_BUSY & (RAMSHIELD_SHM_TABLE_CAPACITY - 1u));
    RamshieldShmRuleEntry *slot = &table[index];
    atomic_store_explicit(&slot->seq, 1u, memory_order_release);

    const RamshieldShmLookup r = ramshield_shm_read_ex(table, HASH_BUSY, now, &snap);
    CHECK(r == RAMSHIELD_SHM_CONTENDED,
          "a permanently odd sequence must be CONTENDED, not MISS; got %d", (int)r);
    CHECK(r != RAMSHIELD_SHM_MISS,
          "a contended lookup must never masquerade as a coherent miss");
}

/* A sequence counter that changes on every attempt is the other half of the
 * same defect. The reader may still win a stable window and find the rule,
 * but it must never conclude that the rule is absent. */
static _Atomic bool stop_churner;

static void *seq_churner(void *opaque)
{
    RamshieldShmRuleEntry *slot = opaque;
    uint32_t v = 2u;
    while (!atomic_load_explicit(&stop_churner, memory_order_acquire)) {
        v += 2u; /* even values only: the slot looks stable but keeps moving */
        atomic_store_explicit(&slot->seq, v, memory_order_release);
    }
    return NULL;
}

static void test_changing_sequence_is_never_a_miss(void)
{
    RamshieldShmRuleSnapshot snap;
    reset_table();
    const uint64_t now = 1000;
    publish(HASH_BUSY, now + 60000);

    const size_t index = (size_t)(HASH_BUSY & (RAMSHIELD_SHM_TABLE_CAPACITY - 1u));
    RamshieldShmRuleEntry *slot = &table[index];

    atomic_store_explicit(&stop_churner, false, memory_order_release);
    pthread_t churner;
    if (pthread_create(&churner, NULL, seq_churner, slot) != 0) {
        fputs("SHM contract: churner thread creation failed\n", stderr);
        failures++;
        return;
    }

    int misses = 0;
    int found = 0;
    int contended = 0;
    for (unsigned i = 0; i < 200u; ++i) {
        const RamshieldShmLookup r = ramshield_shm_read_ex(table, HASH_BUSY, now, &snap);
        if (r == RAMSHIELD_SHM_MISS) {
            misses++;
        } else if (r == RAMSHIELD_SHM_FOUND) {
            found++;
            /* A rule that was read must be coherent, never torn. */
            CHECK(snap.client_hash == HASH_BUSY, "changing sequence: torn client_hash");
            CHECK(snap.expires_at_ms == now + 60000, "changing sequence: torn expiry");
        } else if (r == RAMSHIELD_SHM_CONTENDED) {
            contended++;
        }
    }

    atomic_store_explicit(&stop_churner, true, memory_order_release);
    pthread_join(churner, NULL);

    CHECK(misses == 0,
          "a changing sequence must never be reported as a coherent miss (%d misses)", misses);
    CHECK(found + contended == 200, "every lookup must be FOUND or CONTENDED");
}

/* Prefix lengths. The table reserves key 0 as its empty-slot sentinel, and
 * a /0 prefix hashes to exactly that key, so it cannot be stored or found;
 * a prefix above 32 is not an IPv4 prefix at all. Neither may be routed
 * into the table. The rule published under the reserved key below is the
 * trap: if an invalid prefix were looked up as key 0, that rule would be
 * FOUND. */
static void test_prefix_boundaries(void)
{
    RamshieldShmRuleSnapshot snap;
    reset_table();
    const uint64_t now = 1000;
    const uint32_t client = 0xC6336401u; /* 198.51.100.1 */
    publish(HASH_ZERO, now + 60000);

    const RamshieldShmLookup p0 = ramshield_shm_read_ipv4_ex(table, client, 0, now, &snap);
    const RamshieldShmLookup p24 = ramshield_shm_read_ipv4_ex(table, client, 24, now, &snap);
    const RamshieldShmLookup p32 = ramshield_shm_read_ipv4_ex(table, client, 32, now, &snap);
    const RamshieldShmLookup p33 = ramshield_shm_read_ipv4_ex(table, client, 33, now, &snap);
    const RamshieldShmLookup p255 = ramshield_shm_read_ipv4_ex(table, client, 255, now, &snap);

    CHECK(p0 != RAMSHIELD_SHM_FOUND,
          "prefix 0 must not be looked up as the reserved key 0 (got %d)", (int)p0);
    CHECK(p0 == RAMSHIELD_SHM_INVALID,
          "prefix 0 is not representable in this table; expected INVALID, got %d", (int)p0);
    CHECK(p33 == RAMSHIELD_SHM_INVALID,
          "prefix 33 is out of range; expected INVALID, got %d", (int)p33);
    CHECK(p255 == RAMSHIELD_SHM_INVALID,
          "prefix 255 is out of range; expected INVALID, got %d", (int)p255);
    CHECK(p24 != RAMSHIELD_SHM_INVALID, "prefix 24 is representable");
    CHECK(p32 != RAMSHIELD_SHM_INVALID, "prefix 32 is representable");

    /* The compatibility wrapper must not report an invalid prefix as a hit. */
    CHECK(!ramshield_shm_read_ipv4(table, client, 33, now, &snap),
          "the bool wrapper must not find a rule for an out-of-range prefix");
    CHECK(!ramshield_shm_read_ipv4(table, client, 0, now, &snap),
          "the bool wrapper must not find a rule for prefix 0");
}

/* The C slot-key hash must agree with the Rust one. If they diverge, a rule
 * published by the Rust writer lands in a slot the C reader never looks in,
 * and every rule is silently unenforced. The same vectors are pinned by the
 * Rust test `c_and_rust_slot_key_hash_agree`, so a change to either side
 * breaks one of the two tests. */
static void test_key_parity_with_rust(void)
{
    CHECK(ramshield_subnet_key(0xC0000200u, 24) == UINT64_C(0xff306298),
          "C subnet_key disagrees with the Rust vector (network 192.0.2.0/24)");
    CHECK(ramshield_subnet_key(0xC0000200u, 0) == UINT64_C(0xab616229),
          "C subnet_key disagrees with the Rust vector (prefix 0)");
    CHECK(ramshield_ipv4_subnet_key(0xC6336401u, 24) == UINT64_C(0x7601e635),
          "C ipv4 subnet_key disagrees with the Rust vector (198.51.100.1/24)");
    CHECK(ramshield_ipv4_subnet_key(0xC6336401u, 32) == UINT64_C(0x4c38342d),
          "C ipv4 subnet_key disagrees with the Rust vector (198.51.100.1/32)");
    CHECK(ramshield_ipv4_subnet_key(0xC6336401u, 0) == 0,
          "a /0 prefix must map to the reserved key 0, not to a usable key");
}

/* A live rule sitting BEHIND a slot that was cleared.
 *
 * The writer places a rule in the first free slot, so under the in-repo
 * writer a live rule can never sit behind an empty one. But
 * ramshield_shm_clear_slot() can empty a slot, and a C-side writer that
 * clears a slot in front of a live rule would leave the rule unreachable if
 * the reader stopped at the first empty slot. The reader must scan the
 * whole probe window, which is also what the reader did before this change
 * -- its "empty terminates the probe" comment did not describe the code. */
static void test_live_rule_behind_a_cleared_slot_is_still_found(void)
{
    RamshieldShmRuleSnapshot snap;
    reset_table();
    const uint64_t now = 1000;

    /* The live rule goes in the SECOND probe slot, and the first slot is
     * left empty -- the shape a clearing writer produces. */
    const size_t first = (size_t)(HASH_HIT & (RAMSHIELD_SHM_TABLE_CAPACITY - 1u));
    const size_t second = first + 1u;
    publish_into(second, HASH_HIT, now + 60000);

    const RamshieldShmLookup r = ramshield_shm_read_ex(table, HASH_HIT, now, &snap);
    CHECK(r == RAMSHIELD_SHM_FOUND,
          "a live rule behind a cleared slot must still be found; got %d", (int)r);
    CHECK(first != second, "the test must use two distinct slots");
}

/* A lookup that does not find a rule must not write into `out`: a caller
 * that reuses one snapshot across lookups must not read a stale rule as if
 * it were this lookup's result. */
static void test_snapshot_is_untouched_when_no_rule_is_found(void)
{
    RamshieldShmRuleSnapshot snap;
    reset_table();
    const uint64_t now = 1000;
    publish(HASH_HIT, now + 60000);

    /* Seed the snapshot with a real rule, then look up a different key. */
    RamshieldShmRuleSnapshot seeded;
    CHECK(ramshield_shm_read_ex(table, HASH_HIT, now, &seeded) == RAMSHIELD_SHM_FOUND,
          "the seeding lookup must find the rule");

    memcpy(&snap, &seeded, sizeof(snap));
    const RamshieldShmLookup r = ramshield_shm_read_ex(table, HASH_MISS, now, &snap);
    CHECK(r == RAMSHIELD_SHM_MISS, "the second lookup must miss; got %d", (int)r);
    CHECK(snap.client_hash == HASH_HIT,
          "a lookup that found nothing must not write into the snapshot");
}

/* The boolean compatibility wrapper must still report "not found" for a
 * contended lookup -- that is the behaviour it has always had, and the
 * reason the explicit-result form exists. */
static void test_bool_wrapper_reports_false_under_contention(void)
{
    RamshieldShmRuleSnapshot snap;
    reset_table();
    const uint64_t now = 1000;
    publish(HASH_BUSY, now + 60000);

    const size_t index = (size_t)(HASH_BUSY & (RAMSHIELD_SHM_TABLE_CAPACITY - 1u));
    RamshieldShmRuleEntry *slot = &table[index];
    atomic_store_explicit(&slot->seq, 1u, memory_order_release);

    CHECK(!ramshield_shm_read(table, HASH_BUSY, now, &snap),
          "the bool wrapper must report false for a contended lookup");
}

/* The reserved key cannot be looked up: key 0 is the empty-slot sentinel,
 * so a lookup for it could neither find a rule nor prove one absent. */
static void test_reserved_key_is_rejected(void)
{
    RamshieldShmRuleSnapshot snap;
    reset_table();
    const uint64_t now = 1000;
    publish(HASH_ZERO, now + 60000);

    const RamshieldShmLookup r = ramshield_shm_read_ex(table, HASH_ZERO, now, &snap);
    CHECK(r == RAMSHIELD_SHM_INVALID,
          "the reserved key must be rejected, not looked up; got %d", (int)r);
    CHECK(!ramshield_shm_read(table, HASH_ZERO, now, &snap),
          "the bool wrapper must not find a rule under the reserved key");
}


/* Two rules whose keys share a primary slot.
 *
 * The publisher places a colliding rule in the next slot of the probe
 * window. The reader must find each of them, and must not report the
 * neighbour's rule as the one that was asked for. */
static void test_colliding_keys_are_each_found(void)
{
    RamshieldShmRuleSnapshot snap;
    reset_table();
    const uint64_t now = 1000;

    const size_t primary = (size_t)(HASH_COLLIDE_A & (RAMSHIELD_SHM_TABLE_CAPACITY - 1u));
    CHECK(primary == (size_t)(HASH_COLLIDE_B & (RAMSHIELD_SHM_TABLE_CAPACITY - 1u)),
          "the two keys must share a primary slot for this test to mean anything");
    publish_into(primary, HASH_COLLIDE_A, now + 60000);
    publish_into(primary + 1u, HASH_COLLIDE_B, now + 60000);

    CHECK(ramshield_shm_read_ex(table, HASH_COLLIDE_A, now, &snap) == RAMSHIELD_SHM_FOUND,
          "the first colliding rule must be found");
    CHECK(snap.client_hash == HASH_COLLIDE_A,
          "the first lookup must return the first rule, not its neighbour");

    CHECK(ramshield_shm_read_ex(table, HASH_COLLIDE_B, now, &snap) == RAMSHIELD_SHM_FOUND,
          "the second colliding rule must be found");
    CHECK(snap.client_hash == HASH_COLLIDE_B,
          "the second lookup must return the second rule, not its neighbour");
}

int main(void)
{
    const size_t bytes = (size_t)RAMSHIELD_SHM_TABLE_CAPACITY * sizeof(*table);
    table = aligned_alloc(_Alignof(RamshieldShmRuleEntry), bytes);
    if (table == NULL) {
        fputs("SHM contract: aligned allocation failed\n", stderr);
        return 2;
    }
    memset(table, 0, bytes);
    atomic_init(&stop_churner, false);

    test_hit();
    test_coherent_miss();
    test_expired_rule_is_a_miss();
    test_permanently_odd_sequence_is_contended();
    test_changing_sequence_is_never_a_miss();
    test_prefix_boundaries();
    test_key_parity_with_rust();
    test_live_rule_behind_a_cleared_slot_is_still_found();
    test_snapshot_is_untouched_when_no_rule_is_found();
    test_bool_wrapper_reports_false_under_contention();
    test_reserved_key_is_rejected();
    test_colliding_keys_are_each_found();

    free(table);
    if (failures != 0) {
        fprintf(stderr, "SHM lookup contract: FAIL (%d checks failed)\n", failures);
        return 1;
    }
    puts("SHM lookup contract: PASS");
    return 0;
}
