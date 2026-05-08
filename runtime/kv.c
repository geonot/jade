/* ── Jinn KV Store Runtime ────────────────────────────────────────
 *  In-memory hash map with disk persistence.
 *  Keys: null-terminated strings (max 255 bytes).
 *  Values: i64 (8 bytes).
 *  File format: [8B magic][8B count][entries...]
 *    Entry: [8B hash][256B key (null-padded)][8B value][8B status]
 * ────────────────────────────────────────────────────────────────── */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include "jinn_rt.h"

#define KV_MAGIC      "JADEKV\0\0"
#define KV_MAGIC_SIZE 8
#define KV_KEY_SIZE   256
#define KV_INIT_CAP   64
#define KV_LOAD_MAX   0.7

#define KV_EMPTY     0
#define KV_OCCUPIED  1
#define KV_TOMBSTONE 2

typedef struct {
    uint64_t hash;
    char     key[KV_KEY_SIZE];
    int64_t  value;
    int64_t  status;
} KvSlot;

struct JinnKV {
    FILE    *fp;
    KvSlot  *slots;
    int64_t  capacity;
    int64_t  count;
};

/* ── FNV-1a hash ──────────────────────────────────────────────── */

static uint64_t kv_hash(const char *key, int64_t len) {
    uint64_t h = 14695981039346656037ULL;
    for (int64_t i = 0; i < len; i++) {
        h ^= (uint64_t)(unsigned char)key[i];
        h *= 1099511628211ULL;
    }
    return h;
}

/* ── Internal helpers ─────────────────────────────────────────── */

static int64_t kv_find_slot(JinnKV *kv, uint64_t hash, const char *key, int64_t key_len) {
    int64_t mask = kv->capacity - 1;
    int64_t slot = (int64_t)(hash & (uint64_t)mask);
    for (;;) {
        KvSlot *s = &kv->slots[slot];
        if (s->status == KV_EMPTY) return -(slot + 1); /* not found, return insert pos (negated, 1-based) */
        if (s->status == KV_OCCUPIED && s->hash == hash) {
            int64_t slen = (int64_t)strnlen(s->key, KV_KEY_SIZE);
            if (slen == key_len && memcmp(s->key, key, (size_t)key_len) == 0) {
                return slot; /* found */
            }
        }
        slot = (slot + 1) & mask;
    }
}

static void kv_grow(JinnKV *kv) {
    int64_t old_cap = kv->capacity;
    KvSlot *old_slots = kv->slots;

    kv->capacity = old_cap * 2;
    kv->slots = (KvSlot *)calloc((size_t)kv->capacity, sizeof(KvSlot));
    if (!kv->slots) {
        kv->capacity = old_cap;
        kv->slots = old_slots;
        return;
    }
    kv->count = 0;

    for (int64_t i = 0; i < old_cap; i++) {
        if (old_slots[i].status == KV_OCCUPIED) {
            int64_t key_len = (int64_t)strnlen(old_slots[i].key, KV_KEY_SIZE);
            int64_t slot = kv_find_slot(kv, old_slots[i].hash, old_slots[i].key, key_len);
            if (slot < 0) slot = -(slot + 1); /* decode insert position */
            kv->slots[slot] = old_slots[i];
            kv->count++;
        }
    }
    free(old_slots);
}

static void kv_save(JinnKV *kv) {
    if (!kv->fp) return;
    if (fseek(kv->fp, 0, SEEK_SET) != 0) {
        fprintf(stderr, "jinn: kv: fseek failed during save\n");
        return;
    }

    char magic[KV_MAGIC_SIZE];
    memcpy(magic, KV_MAGIC, KV_MAGIC_SIZE);
    if (fwrite(magic, 1, KV_MAGIC_SIZE, kv->fp) != KV_MAGIC_SIZE ||
        fwrite(&kv->count, sizeof(int64_t), 1, kv->fp) != 1) {
        fprintf(stderr, "jinn: kv: write header failed\n");
        return;
    }

    /* Write only occupied entries */
    for (int64_t i = 0; i < kv->capacity; i++) {
        if (kv->slots[i].status == KV_OCCUPIED) {
            if (fwrite(&kv->slots[i], sizeof(KvSlot), 1, kv->fp) != 1) {
                fprintf(stderr, "jinn: kv: write entry failed\n");
                return;
            }
        }
    }
    fflush(kv->fp);
}

/* ── Public API ───────────────────────────────────────────────── */

JinnKV *jinn_kv_open(const char *path) {
    JinnKV *kv = (JinnKV *)calloc(1, sizeof(JinnKV));
    kv->capacity = KV_INIT_CAP;
    kv->slots = (KvSlot *)calloc((size_t)kv->capacity, sizeof(KvSlot));
    kv->count = 0;

    kv->fp = fopen(path, "r+b");
    if (kv->fp) {
        /* Load existing data */
        char magic[KV_MAGIC_SIZE];
        if (fread(magic, 1, KV_MAGIC_SIZE, kv->fp) == KV_MAGIC_SIZE
            && memcmp(magic, KV_MAGIC, KV_MAGIC_SIZE) == 0) {

            int64_t entry_count = 0;
            fread(&entry_count, sizeof(int64_t), 1, kv->fp);

            /* Ensure capacity */
            while ((double)(entry_count + 1) / (double)kv->capacity > KV_LOAD_MAX) {
                int64_t new_cap = kv->capacity * 2;
                free(kv->slots);
                kv->capacity = new_cap;
                kv->slots = (KvSlot *)calloc((size_t)kv->capacity, sizeof(KvSlot));
            }

            /* Read entries and insert into hash table */
            for (int64_t i = 0; i < entry_count; i++) {
                KvSlot entry;
                if (fread(&entry, sizeof(KvSlot), 1, kv->fp) != 1) break;
                entry.status = KV_OCCUPIED;

                int64_t key_len = (int64_t)strnlen(entry.key, KV_KEY_SIZE);
                int64_t slot = kv_find_slot(kv, entry.hash, entry.key, key_len);
                if (slot < 0) slot = -(slot + 1);
                kv->slots[slot] = entry;
                kv->count++;
            }
        }
    } else {
        /* Create new file */
        kv->fp = fopen(path, "w+b");
        if (kv->fp) {
            kv_save(kv);
        }
    }
    return kv;
}

void jinn_kv_close(JinnKV *kv) {
    if (!kv) return;
    kv_save(kv);
    if (kv->fp) fclose(kv->fp);
    free(kv->slots);
    free(kv);
}

void jinn_kv_set(JinnKV *kv, const char *key, int64_t key_len, int64_t value) {
    if (!kv || !key || key_len <= 0) return;
    if (key_len >= KV_KEY_SIZE) key_len = KV_KEY_SIZE - 1;

    uint64_t hash = kv_hash(key, key_len);
    int64_t slot = kv_find_slot(kv, hash, key, key_len);

    if (slot >= 0) {
        /* Update existing */
        kv->slots[slot].value = value;
    } else {
        /* Insert new */
        if ((double)(kv->count + 1) / (double)kv->capacity > KV_LOAD_MAX) {
            kv_grow(kv);
            slot = kv_find_slot(kv, hash, key, key_len);
            if (slot < 0) slot = -(slot + 1);
        } else {
            slot = -(slot + 1);
        }
        kv->slots[slot].hash = hash;
        memset(kv->slots[slot].key, 0, KV_KEY_SIZE);
        memcpy(kv->slots[slot].key, key, (size_t)key_len);
        kv->slots[slot].value = value;
        kv->slots[slot].status = KV_OCCUPIED;
        kv->count++;
    }
    kv_save(kv);
}

int64_t jinn_kv_get(JinnKV *kv, const char *key, int64_t key_len) {
    if (!kv || !key || key_len <= 0) return 0;
    if (key_len >= KV_KEY_SIZE) key_len = KV_KEY_SIZE - 1;

    uint64_t hash = kv_hash(key, key_len);
    int64_t slot = kv_find_slot(kv, hash, key, key_len);
    if (slot >= 0) return kv->slots[slot].value;
    return 0; /* not found → return 0 */
}

int jinn_kv_has(JinnKV *kv, const char *key, int64_t key_len) {
    if (!kv || !key || key_len <= 0) return 0;
    if (key_len >= KV_KEY_SIZE) key_len = KV_KEY_SIZE - 1;

    uint64_t hash = kv_hash(key, key_len);
    int64_t slot = kv_find_slot(kv, hash, key, key_len);
    return slot >= 0 ? 1 : 0;
}

void jinn_kv_del(JinnKV *kv, const char *key, int64_t key_len) {
    if (!kv || !key || key_len <= 0) return;
    if (key_len >= KV_KEY_SIZE) key_len = KV_KEY_SIZE - 1;

    uint64_t hash = kv_hash(key, key_len);
    int64_t slot = kv_find_slot(kv, hash, key, key_len);
    if (slot >= 0) {
        kv->slots[slot].status = KV_TOMBSTONE;
        kv->count--;
        kv_save(kv);
    }
}

void jinn_kv_incr(JinnKV *kv, const char *key, int64_t key_len, int64_t delta) {
    if (!kv || !key || key_len <= 0) return;
    if (key_len >= KV_KEY_SIZE) key_len = KV_KEY_SIZE - 1;

    uint64_t hash = kv_hash(key, key_len);
    int64_t slot = kv_find_slot(kv, hash, key, key_len);

    if (slot >= 0) {
        kv->slots[slot].value += delta;
    } else {
        /* Key doesn't exist — create with delta as initial value */
        if ((double)(kv->count + 1) / (double)kv->capacity > KV_LOAD_MAX) {
            kv_grow(kv);
            slot = kv_find_slot(kv, hash, key, key_len);
            if (slot < 0) slot = -(slot + 1);
        } else {
            slot = -(slot + 1);
        }
        kv->slots[slot].hash = hash;
        memset(kv->slots[slot].key, 0, KV_KEY_SIZE);
        memcpy(kv->slots[slot].key, key, (size_t)key_len);
        kv->slots[slot].value = delta;
        kv->slots[slot].status = KV_OCCUPIED;
        kv->count++;
    }
    kv_save(kv);
}

int64_t jinn_kv_count(JinnKV *kv) {
    if (!kv) return 0;
    return kv->count;
}

void jinn_kv_persist(JinnKV *kv) {
    if (!kv) return;
    kv_save(kv);
}
