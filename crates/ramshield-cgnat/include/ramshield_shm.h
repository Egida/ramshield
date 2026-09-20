#ifndef RAMSHIELD_SHM_H
#define RAMSHIELD_SHM_H

#include <stdint.h>
#include <stdbool.h>
#include <stdatomic.h>

#define RAMSHIELD_SHM_TABLE_CAPACITY 65536
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
    uint32_t mask = prefix_len == 0 ? 0 : 0xffffffffU << (32 - prefix_len);
    return ramshield_subnet_key(client_ip & mask, prefix_len);
}


/* Must match Rust ShmRuleEntry byte-for-byte. */
typedef struct __attribute__((aligned(64))) {
    _Atomic uint32_t seq;       /* odd = writer active, even = stable */
    _Atomic uint64_t client_hash;
    _Atomic uint64_t expires_at_ms;
    _Atomic uint16_t max_rps;
    _Atomic uint8_t  tier;
    _Atomic uint8_t  flags;
    uint8_t          challenge_seed[16];
    uint8_t          _padding[26];
} RamshieldShmRuleEntry;

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
 * Read a coherent rule snapshot. The caller must use the copied snapshot,
 * never retain a pointer into the mmap: a writer may begin immediately after
 * this function returns.
 */
static inline bool
ramshield_shm_read(const RamshieldShmRuleEntry *table,
                   uint64_t hash,
                   uint64_t now_ms,
                   RamshieldShmRuleSnapshot *out)
{
    uint32_t idx = (uint32_t)(hash & (RAMSHIELD_SHM_TABLE_CAPACITY - 1));
    const RamshieldShmRuleEntry *entry = &table[idx];

    for (unsigned attempt = 0; attempt < 64; ++attempt) {
        uint32_t before = atomic_load_explicit(&entry->seq, memory_order_acquire);
        if (before & 1u) continue;

        uint64_t client_hash = atomic_load_explicit(&entry->client_hash, memory_order_relaxed);
        uint64_t expires_at_ms = atomic_load_explicit(&entry->expires_at_ms, memory_order_relaxed);
        uint16_t max_rps = atomic_load_explicit(&entry->max_rps, memory_order_relaxed);
        uint8_t tier = atomic_load_explicit(&entry->tier, memory_order_relaxed);
        uint8_t flags = atomic_load_explicit(&entry->flags, memory_order_relaxed);
        uint8_t challenge_seed[16];
        for (unsigned i = 0; i < sizeof(challenge_seed); ++i)
            challenge_seed[i] = entry->challenge_seed[i];

        /* Fence: the payload reads (and the plain challenge_seed bytes) must
         * complete before the seq re-read, or the re-read can pass while the
         * values were observed mid-write. Canonical C11 seqlock reader: the
         * re-read itself may then be relaxed. */
        atomic_thread_fence(memory_order_acquire);
        uint32_t after = atomic_load_explicit(&entry->seq, memory_order_relaxed);
        if (before != after || (after & 1u)) continue;
        if (client_hash != hash || expires_at_ms <= now_ms) return false;

        out->client_hash = client_hash;
        out->expires_at_ms = expires_at_ms;
        out->max_rps = max_rps;
        out->tier = tier;
        out->flags = flags;
        for (unsigned i = 0; i < sizeof(challenge_seed); ++i)
            out->challenge_seed[i] = challenge_seed[i];
        return true;
    }
    return false;
}

static inline bool
ramshield_shm_read_ipv4(const RamshieldShmRuleEntry *table,
                        uint32_t client_ip,
                        uint8_t prefix_len,
                        uint64_t now_ms,
                        RamshieldShmRuleSnapshot *out)
{
    return ramshield_shm_read(
        table,
        ramshield_ipv4_subnet_key(client_ip, prefix_len),
        now_ms,
        out);
}

/* Clears entries through the same publication protocol. */
static inline void
ramshield_shm_flush_all(RamshieldShmRuleEntry *table, uint64_t now_ms)
{
    for (uint32_t i = 0; i < RAMSHIELD_SHM_TABLE_CAPACITY; i++) {
        /* Relaxed is correct for the odd marker itself — but the marker must
         * be VISIBLE before the payload stores, or a reader can observe a
         * half-published entry while seq still reads even (before == after)
         * and accept the tear. The audit's fix (release ON this increment)
         * orders the previous cycle's stores, not the payload after it; the
         * release FENCE here is what pairs with the reader's acquire fence. */
        atomic_fetch_add_explicit(&table[i].seq, 1, memory_order_relaxed);
        atomic_thread_fence(memory_order_release);
        atomic_store_explicit(&table[i].client_hash, 0, memory_order_relaxed);
        atomic_store_explicit(&table[i].expires_at_ms, now_ms, memory_order_relaxed);
        atomic_store_explicit(&table[i].tier, RAMSHIELD_TIER_ALLOW, memory_order_relaxed);
        atomic_fetch_add_explicit(&table[i].seq, 1, memory_order_release);
    }
}

#endif // RAMSHIELD_SHM_H
