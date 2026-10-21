#ifndef RAMSHIELD_SHM_H
#define RAMSHIELD_SHM_H

#include <stdint.h>
#include <stdbool.h>
#include <stdatomic.h>
#include <stddef.h>

#define RAMSHIELD_SHM_TABLE_CAPACITY 262144
#define RAMSHIELD_SHM_PROBE_LIMIT 4
#define RAMSHIELD_FLAG_SHARED_INFRA 0x01

static inline uint64_t
ramshield_subnet_key(uint32_t network, uint8_t prefix_len)
{
    uint32_t h = network ^ (uint32_t)prefix_len;
    h ^= h >> 16;
    h *= 0x85ebca6bU;
    h ^= h >> 13;
    h *= 0xc2b2ae35U;
    h ^= h >> 16;
    return (uint64_t)h;
}

static inline uint64_t
ramshield_ipv4_subnet_key(uint32_t client_ip, uint8_t prefix_len)
{
    if (prefix_len > 32 || prefix_len == 0) {
        /* A prefix this table cannot represent. Both cases hash to the
         * reserved key 0: an out-of-range prefix is rejected outright, and a
         * /0 prefix masks to network 0, which hashes to 0. Key 0 is the
         * empty-slot sentinel, so a rule stored under it is invisible to
         * every reader. Do not use the result of this helper as a key for
         * an unrepresentable prefix -- use ramshield_shm_read_ipv4_ex, which
         * reports it as INVALID. */
        return 0;
    }
    uint32_t mask = prefix_len == 0 ? 0 : 0xffffffffU << (32 - prefix_len);
    return ramshield_subnet_key(client_ip & mask, prefix_len);
}


/* Must match Rust ShmRuleEntry byte-for-byte. */
typedef struct __attribute__((aligned(64))) {
    _Atomic uint32_t seq;           /* odd = writer active, even = stable */
    _Atomic uint64_t client_hash;   /* offset 8  (4 bytes padding after seq) */
    _Atomic uint64_t expires_at_ms; /* offset 16 */
    /* Two naturally aligned atomic words replace the former byte array:
     * a 16-byte memcpy under a seqlock is a torn-read/UB hazard for a
     * non-atomic object while a writer runs (C11 data race). */
    _Atomic uint64_t challenge_seed_lo; /* offset 24 */
    _Atomic uint64_t challenge_seed_hi; /* offset 32 */
    _Atomic uint16_t max_rps;       /* offset 40 */
    _Atomic uint8_t  tier;          /* offset 42 */
    _Atomic uint8_t  flags;         /* offset 43 */
    uint8_t          _padding[20];  /* offset 44..64 */
} RamshieldShmRuleEntry;

_Static_assert(_Alignof(RamshieldShmRuleEntry) == 64,
               "RamshieldShmRuleEntry ABI alignment must remain 64 bytes");
_Static_assert(sizeof(RamshieldShmRuleEntry) == 64,
               "RamshieldShmRuleEntry ABI must remain exactly 64 bytes");
_Static_assert(offsetof(RamshieldShmRuleEntry, seq) == 0, "seq offset changed");
_Static_assert(offsetof(RamshieldShmRuleEntry, client_hash) == 8, "client_hash offset changed");
_Static_assert(offsetof(RamshieldShmRuleEntry, expires_at_ms) == 16, "expires_at_ms offset changed");
_Static_assert(offsetof(RamshieldShmRuleEntry, challenge_seed_lo) == 24, "challenge_seed_lo offset changed");
_Static_assert(offsetof(RamshieldShmRuleEntry, challenge_seed_hi) == 32, "challenge_seed_hi offset changed");
_Static_assert(offsetof(RamshieldShmRuleEntry, max_rps) == 40, "max_rps offset changed");
_Static_assert(offsetof(RamshieldShmRuleEntry, tier) == 42, "tier offset changed");
_Static_assert(offsetof(RamshieldShmRuleEntry, flags) == 43, "flags offset changed");
_Static_assert(offsetof(RamshieldShmRuleEntry, _padding) == 44, "padding offset changed");

typedef struct {
    uint64_t client_hash;
    uint64_t expires_at_ms;
    uint16_t max_rps;
    uint8_t  tier;
    uint8_t  flags;
    uint8_t  challenge_seed[16];
} RamshieldShmRuleSnapshot;

#define RAMSHIELD_TIER_ALLOW       0u
#define RAMSHIELD_TIER_CHALLENGE   1u /* 429 + JS/PoW */
#define RAMSHIELD_TIER_XDP_DROP    2u
#define RAMSHIELD_TIER_BLOCK       3u

/*
 * Result of a rule lookup.
 *
 * FOUND     - a coherent, live rule was copied into `out`.
 * MISS      - the lookup completed coherently and no live rule matched.
 * CONTENDED - the lookup could not obtain a coherent snapshot within the
 *              bounded retry budget. This is NOT a miss: the rule may well
 *              be present in the slot that could not be read. A caller that
 *              treats CONTENDED as a miss fails OPEN, which for a mitigation
 *              table means a challenge or a block is skipped.
 *
 *              The response is deployment-specific -- retry the lookup, or
 *              apply the policy that applies to a client with no known rule.
 *              What is always wrong is treating it as a coherent miss.
 * INVALID   - the input was rejected before any lookup was performed: a
 *              prefix length that is out of range, or one that the table
 *              cannot represent (a /0 prefix hashes to the reserved empty
 *              key), or the reserved key itself. No slot was read.
 *
 * Always compare the result against the named constants. A truthiness test
 * is wrong: RAMSHIELD_SHM_FOUND is zero, so `if (result)` reads as failure
 * for a successful lookup.
 */
typedef enum {
    RAMSHIELD_SHM_FOUND = 0,
    RAMSHIELD_SHM_MISS = 1,
    RAMSHIELD_SHM_CONTENDED = 2,
    RAMSHIELD_SHM_INVALID = 3
} RamshieldShmLookup;

/* The result values are part of the published contract: an out-of-repo
 * consumer may compare them, store them, or persist them. Pin the values so
 * that a reordering cannot silently change their meaning.
 *
 * The underlying type of an enum is implementation-defined. A consumer that
 * keeps a result beyond the call -- in a struct, on disk, or across a
 * process boundary -- should store it as an int, not as the enum type. */
_Static_assert(RAMSHIELD_SHM_FOUND == 0,
               "the FOUND value is part of the published contract");
_Static_assert(RAMSHIELD_SHM_MISS == 1,
               "the MISS value is part of the published contract");
_Static_assert(RAMSHIELD_SHM_CONTENDED == 2,
               "the CONTENDED value is part of the published contract");
_Static_assert(RAMSHIELD_SHM_INVALID == 3,
               "the INVALID value is part of the published contract");

/*
 * Read a coherent rule snapshot. The caller must use the copied snapshot,
 * never retain a pointer into the mmap: a writer may begin immediately after
 * this function returns.
 *
 * `out` is written only on RAMSHIELD_SHM_FOUND. A lookup that does not
 * find a rule leaves it untouched, so a caller that reuses one snapshot
 * across lookups cannot read a previous rule as this lookup's result.
 *
 * This is the explicit-result form. Use it when a contended read must not be
 * mistaken for an absent rule.
 */
static inline RamshieldShmLookup
ramshield_shm_read_ex(const RamshieldShmRuleEntry *table,
                      uint64_t hash,
                      uint64_t now_ms,
                      RamshieldShmRuleSnapshot *out)
{
    /* Key 0 is the reserved empty-slot sentinel. A slot whose client_hash is
     * 0 is empty, so a lookup for key 0 could never find a rule and could
     * never prove one absent either. Reject it rather than report a miss
     * for a key that cannot exist.
     *
     * Key 0 is not only the /0 case: the slot-key mixer is 0-preserving, so
     * a key is 0 exactly when the masked network address equals its prefix
     * length -- every /0, and 0.0.0.32/32 among others. The publisher
     * refuses all of them the same way. */
    if (hash == 0) {
        return RAMSHIELD_SHM_INVALID;
    }
    uint32_t primary = (uint32_t)(hash & (RAMSHIELD_SHM_TABLE_CAPACITY - 1));
    bool contended = false;

    for (unsigned probe = 0; probe < RAMSHIELD_SHM_PROBE_LIMIT; ++probe) {
        const RamshieldShmRuleEntry *entry =
            &table[(primary + probe) & (RAMSHIELD_SHM_TABLE_CAPACITY - 1)];
        /* Did this slot yield an authoritative snapshot? A slot that was
         * read coherently is known; a slot whose retry budget ran out is
         * not, and it may hold the rule we are looking for. */
        bool coherent = false;

        for (unsigned attempt = 0; attempt < 64; ++attempt) {
            uint32_t before = atomic_load_explicit(&entry->seq, memory_order_acquire);
            if (before & 1u) continue;

            uint64_t client_hash = atomic_load_explicit(&entry->client_hash, memory_order_relaxed);
            uint64_t expires_at_ms = atomic_load_explicit(&entry->expires_at_ms, memory_order_relaxed);
            uint16_t max_rps = atomic_load_explicit(&entry->max_rps, memory_order_relaxed);
            uint8_t tier = atomic_load_explicit(&entry->tier, memory_order_relaxed);
            uint8_t flags = atomic_load_explicit(&entry->flags, memory_order_relaxed);
            /* Atomic loads, not a byte copy: the seed is two 64-bit words so
             * a reader never observes a mix of two writer generations. */
            uint64_t challenge_seed_lo = atomic_load_explicit(&entry->challenge_seed_lo, memory_order_relaxed);
            uint64_t challenge_seed_hi = atomic_load_explicit(&entry->challenge_seed_hi, memory_order_relaxed);

            atomic_thread_fence(memory_order_acquire);
            uint32_t after = atomic_load_explicit(&entry->seq, memory_order_acquire);
            if (before != after || (after & 1u)) continue;

            coherent = true;
            if (client_hash == hash && expires_at_ms > now_ms) {
                out->client_hash = client_hash;
                out->expires_at_ms = expires_at_ms;
                out->max_rps = max_rps;
                out->tier = tier;
                out->flags = flags;
                for (unsigned i = 0; i < 8; ++i)
                    out->challenge_seed[i]     = (uint8_t)(challenge_seed_lo >> (8 * i));
                for (unsigned i = 0; i < 8; ++i)
                    out->challenge_seed[8 + i] = (uint8_t)(challenge_seed_hi >> (8 * i));
                return RAMSHIELD_SHM_FOUND;
            }

            // The slot was read coherently and holds no live rule for this
            // key: it is either empty, or it holds a different key, or it
            // holds an expired rule. Advance the probe.
            //
            // The probe continues past an empty slot even though the writer
            // places a rule in the FIRST free candidate, and therefore never
            // behind one. ramshield_shm_clear_slot() can empty a slot that
            // has a live rule behind it in the same window, so stopping at
            // the first empty slot would turn that rule into a miss.
            //
            // Expired entries are skipped because an older rule may have
            // expired while a later live collision remains in the bounded
            // probe window.
            break;
        }
        /* The retry budget for this slot was exhausted: a writer held it, or
         * the sequence changed on every attempt. The slot may hold the rule
         * we are looking for, so this is contention, not an absence. A slot
         * that WAS read coherently -- even if it held a different key, or an
         * expired rule -- is known, and does not make the result uncertain. */
        if (!coherent) {
            contended = true;
        }
    }
    return contended ? RAMSHIELD_SHM_CONTENDED : RAMSHIELD_SHM_MISS;
}

/*
 * Compatibility wrapper. A lookup that could not be completed coherently is
 * still reported as "not found", as it always was.
 *
 * The observable behaviour is unchanged with one exception, and it is a
 * case the old code got wrong: an invalid prefix (0, or above 32) is no
 * longer looked up as the reserved key 0, so it can no longer find a rule
 * stored under that key.
 *
 * The whole probe window is scanned, exactly as before this change. The
 * previous comment here said "empty terminates the probe", which did not
 * describe the code: its break ended the inner retry loop, and the probe
 * continued to the next slot.
 *
 * New code should call ramshield_shm_read_ex and handle CONTENDED
 * explicitly; a proxy that treats a contended read as "no rule" skips the
 * challenge or block that rule exists to impose.
 */
static inline bool
ramshield_shm_read(const RamshieldShmRuleEntry *table,
                   uint64_t hash,
                   uint64_t now_ms,
                   RamshieldShmRuleSnapshot *out)
{
    return ramshield_shm_read_ex(table, hash, now_ms, out) == RAMSHIELD_SHM_FOUND;
}

/*
 * IPv4 lookup with an explicit result.
 *
 * A prefix length the table cannot represent is a policy error, not a
 * lookup: it must not be routed into the table as key 0, which is the
 * empty-slot sentinel and would degenerate into a silent miss.
 */
static inline RamshieldShmLookup
ramshield_shm_read_ipv4_ex(const RamshieldShmRuleEntry *table,
                           uint32_t client_ip,
                           uint8_t prefix_len,
                           uint64_t now_ms,
                           RamshieldShmRuleSnapshot *out)
{
    /* Reject before the lookup, not during it.
     *
     * prefix_len == 0 is rejected because a /0 prefix masks to network 0,
     * which hashes to key 0 -- and key 0 is the reserved empty-slot
     * sentinel. A rule for a /0 prefix could be published but never found,
     * so the lookup must say the input is not representable rather than
     * return a silent miss.
     *
     * prefix_len > 32 is not an IPv4 prefix at all. */
    if (prefix_len == 0 || prefix_len > 32) {
        return RAMSHIELD_SHM_INVALID;
    }
    return ramshield_shm_read_ex(
        table,
        ramshield_ipv4_subnet_key(client_ip, prefix_len),
        now_ms,
        out);
}

/* Compatibility wrapper: see ramshield_shm_read. */
static inline bool
ramshield_shm_read_ipv4(const RamshieldShmRuleEntry *table,
                        uint32_t client_ip,
                        uint8_t prefix_len,
                        uint64_t now_ms,
                        RamshieldShmRuleSnapshot *out)
{
    return ramshield_shm_read_ipv4_ex(table, client_ip, prefix_len, now_ms, out)
           == RAMSHIELD_SHM_FOUND;
}

/* Clears one slot using the exclusive even→odd writer protocol (mirrors Rust).
 * Returns false if the seqlock could not be acquired. */
static inline bool
ramshield_shm_clear_slot(RamshieldShmRuleEntry *slot, uint64_t now_ms)
{
    bool acquired = false;
    for (int spin = 0; spin < 64; spin++) {
        uint32_t s = atomic_load_explicit(&slot->seq, memory_order_acquire);
        if (s & 1u) {
            continue; /* another writer holds the lock */
        }
        if (atomic_compare_exchange_strong_explicit(
                &slot->seq, &s, s + 1u,
                memory_order_acq_rel, memory_order_acquire)) {
            acquired = true;
            break;
        }
    }
    if (!acquired) {
        return false;
    }
    atomic_store_explicit(&slot->client_hash, 0, memory_order_relaxed);
    atomic_store_explicit(&slot->expires_at_ms, now_ms, memory_order_relaxed);
    atomic_store_explicit(&slot->max_rps, 0, memory_order_relaxed);
    atomic_store_explicit(&slot->tier, RAMSHIELD_TIER_ALLOW, memory_order_relaxed);
    atomic_store_explicit(&slot->flags, 0, memory_order_relaxed);
    atomic_store_explicit(&slot->challenge_seed_lo, 0, memory_order_relaxed);
    atomic_store_explicit(&slot->challenge_seed_hi, 0, memory_order_relaxed);
    atomic_thread_fence(memory_order_release);
    atomic_fetch_add_explicit(&slot->seq, 1, memory_order_release); /* odd → even */
    return true;
}

/* Clears entries through the exclusive writer protocol. Skips slots that
 * remain locked after the spin budget (best-effort flush). */
static inline void
ramshield_shm_flush_all(RamshieldShmRuleEntry *table, uint64_t now_ms)
{
    for (uint32_t i = 0; i < RAMSHIELD_SHM_TABLE_CAPACITY; i++) {
        (void)ramshield_shm_clear_slot(&table[i], now_ms);
    }
}

#endif // RAMSHIELD_SHM_H
