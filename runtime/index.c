/* runtime/index.c – Hash-index and B-tree-stub helpers for Jade stores.
 *
 * Hash index file layout:
 *   [8B  magic   "JADEIDX\0"]
 *   [8B  capacity (power-of-2 slot count)]
 *   [8B  count   (number of occupied slots)]
 *   [capacity × 24B slots ...]
 *
 * Each slot: [8B hash][8B record_offset][8B status]
 *   status: 0 = empty, 1 = occupied, 2 = tombstone
 *
 * Open-addressing with linear probing.  Grows (2×) when load > 0.7.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

#define IDX_MAGIC     "JADEIDX\0"
#define IDX_MAGIC_LEN 8
#define IDX_HEADER    24          /* magic + capacity + count */
#define SLOT_SIZE     24          /* hash + offset + status */
#define INITIAL_CAP   256
#define STATUS_EMPTY     0
#define STATUS_OCCUPIED  1
#define STATUS_TOMBSTONE 2

/* ── FNV-1a hash for arbitrary bytes ────────────────────────────── */
static uint64_t fnv1a(const void *data, int64_t len) {
    const uint8_t *p = (const uint8_t *)data;
    uint64_t h = 14695981039346656037ULL;
    for (int64_t i = 0; i < len; i++) {
        h ^= p[i];
        h *= 1099511628211ULL;
    }
    return h;
}

/* ── Hash a store field value (i64 or fixed string) ─────────────── */
uint64_t jade_idx_hash_i64(int64_t val) {
    return fnv1a(&val, sizeof(val));
}

uint64_t jade_idx_hash_str(const char *buf, int64_t len) {
    return fnv1a(buf, len);
}

uint64_t jade_idx_hash_f64(double val) {
    return fnv1a(&val, sizeof(val));
}

/* ── Index file management ──────────────────────────────────────── */

typedef struct {
    FILE   *fp;
    int64_t capacity;
    int64_t count;
} JadeIndex;

/* Read header from an open index file */
static int read_header(JadeIndex *idx) {
    fseek(idx->fp, 0, SEEK_SET);
    char mag[IDX_MAGIC_LEN];
    if (fread(mag, 1, IDX_MAGIC_LEN, idx->fp) != IDX_MAGIC_LEN) return -1;
    if (memcmp(mag, IDX_MAGIC, IDX_MAGIC_LEN) != 0) return -1;
    if (fread(&idx->capacity, 8, 1, idx->fp) != 1) return -1;
    if (fread(&idx->count, 8, 1, idx->fp) != 1) return -1;
    return 0;
}

/* Write header */
static int write_header(JadeIndex *idx) {
    if (fseek(idx->fp, 0, SEEK_SET) != 0 ||
        fwrite(IDX_MAGIC, 1, IDX_MAGIC_LEN, idx->fp) != IDX_MAGIC_LEN ||
        fwrite(&idx->capacity, 8, 1, idx->fp) != 1 ||
        fwrite(&idx->count, 8, 1, idx->fp) != 1) {
        fprintf(stderr, "jade: index: write_header failed\n");
        return -1;
    }
    return 0;
}

/* Create a fresh index file with initial capacity */
static void init_file(JadeIndex *idx) {
    idx->capacity = INITIAL_CAP;
    idx->count = 0;
    write_header(idx);
    /* Zero-fill slots */
    uint8_t zero[SLOT_SIZE];
    memset(zero, 0, SLOT_SIZE);
    for (int64_t i = 0; i < INITIAL_CAP; i++) {
        fwrite(zero, 1, SLOT_SIZE, idx->fp);
    }
    fflush(idx->fp);
}

/* Open (or create) an index file.  Returns opaque pointer. */
JadeIndex *jade_idx_open(const char *path) {
    JadeIndex *idx = (JadeIndex *)calloc(1, sizeof(JadeIndex));
    idx->fp = fopen(path, "r+b");
    if (idx->fp && read_header(idx) == 0) {
        return idx;
    }
    /* Create */
    if (idx->fp) fclose(idx->fp);
    idx->fp = fopen(path, "w+b");
    if (!idx->fp) { free(idx); return NULL; }
    init_file(idx);
    return idx;
}

void jade_idx_close(JadeIndex *idx) {
    if (!idx) return;
    if (idx->fp) fclose(idx->fp);
    free(idx);
}

/* ── Slot I/O ───────────────────────────────────────────────────── */

static void read_slot(JadeIndex *idx, int64_t slot,
                      uint64_t *hash, int64_t *offset, int64_t *status) {
    fseek(idx->fp, IDX_HEADER + slot * SLOT_SIZE, SEEK_SET);
    fread(hash, 8, 1, idx->fp);
    fread(offset, 8, 1, idx->fp);
    fread(status, 8, 1, idx->fp);
}

static int write_slot(JadeIndex *idx, int64_t slot,
                      uint64_t hash, int64_t offset, int64_t status) {
    if (fseek(idx->fp, IDX_HEADER + slot * SLOT_SIZE, SEEK_SET) != 0 ||
        fwrite(&hash, 8, 1, idx->fp) != 1 ||
        fwrite(&offset, 8, 1, idx->fp) != 1 ||
        fwrite(&status, 8, 1, idx->fp) != 1) {
        fprintf(stderr, "jade: index: write_slot failed\n");
        return -1;
    }
    return 0;
}

/* ── Grow (rehash) ──────────────────────────────────────────────── */

static void grow(JadeIndex *idx) {
    int64_t old_cap = idx->capacity;

    /* Overflow guard: capacity can never exceed 2^60 slots (prevents
     * integer overflow in old_cap * 2 and slot-count * SLOT_SIZE).   */
    if (old_cap > ((int64_t)1 << 60)) return;

    /* Read all occupied slots */
    typedef struct { uint64_t h; int64_t off; } Entry;
    if (idx->count <= 0 || (size_t)idx->count > SIZE_MAX / sizeof(Entry)) return;
    Entry *entries = (Entry *)malloc(sizeof(Entry) * (size_t)idx->count);
    if (!entries) return;
    int64_t n = 0;
    for (int64_t i = 0; i < old_cap; i++) {
        uint64_t h; int64_t off, st;
        read_slot(idx, i, &h, &off, &st);
        if (st == STATUS_OCCUPIED) {
            entries[n].h = h;
            entries[n].off = off;
            n++;
        }
    }

    /* Double capacity, rewrite file */
    idx->capacity = old_cap * 2;
    idx->count = 0;
    write_header(idx);

    /* Zero-fill new slots */
    uint8_t zero[SLOT_SIZE];
    memset(zero, 0, SLOT_SIZE);
    for (int64_t i = 0; i < idx->capacity; i++) {
        fwrite(zero, 1, SLOT_SIZE, idx->fp);
    }
    fflush(idx->fp);

    /* Re-insert */
    for (int64_t i = 0; i < n; i++) {
        int64_t slot = (int64_t)(entries[i].h & (uint64_t)(idx->capacity - 1));
        for (;;) {
            uint64_t sh; int64_t so, ss;
            read_slot(idx, slot, &sh, &so, &ss);
            if (ss == STATUS_EMPTY) {
                write_slot(idx, slot, entries[i].h, entries[i].off, STATUS_OCCUPIED);
                idx->count++;
                break;
            }
            slot = (slot + 1) & (idx->capacity - 1);
        }
    }
    write_header(idx);
    fflush(idx->fp);
    free(entries);
}

/* ── Insert into index ──────────────────────────────────────────── */

void jade_idx_insert(JadeIndex *idx, uint64_t hash, int64_t record_offset) {
    if (!idx) return;
    /* Check load factor */
    if (idx->count * 10 >= idx->capacity * 7) {
        grow(idx);
    }
    int64_t slot = (int64_t)(hash & (uint64_t)(idx->capacity - 1));
    for (;;) {
        uint64_t sh; int64_t so, ss;
        read_slot(idx, slot, &sh, &so, &ss);
        if (ss == STATUS_EMPTY || ss == STATUS_TOMBSTONE) {
            write_slot(idx, slot, hash, record_offset, STATUS_OCCUPIED);
            idx->count++;
            write_header(idx);
            fflush(idx->fp);
            return;
        }
        slot = (slot + 1) & (idx->capacity - 1);
    }
}

/* ── Lookup: returns record offset or -1 if not found ───────────── */

int64_t jade_idx_lookup(JadeIndex *idx, uint64_t hash) {
    if (!idx) return -1;
    int64_t slot = (int64_t)(hash & (uint64_t)(idx->capacity - 1));
    for (;;) {
        uint64_t sh; int64_t so, ss;
        read_slot(idx, slot, &sh, &so, &ss);
        if (ss == STATUS_EMPTY) return -1;
        if (ss == STATUS_OCCUPIED && sh == hash) return so;
        slot = (slot + 1) & (idx->capacity - 1);
    }
}

/* ── Check if a hash exists (for @unique enforcement) ───────────── */

int jade_idx_contains(JadeIndex *idx, uint64_t hash) {
    return jade_idx_lookup(idx, hash) >= 0 ? 1 : 0;
}

/* ── Delete by hash ─────────────────────────────────────────────── */

void jade_idx_delete(JadeIndex *idx, uint64_t hash) {
    if (!idx) return;
    int64_t slot = (int64_t)(hash & (uint64_t)(idx->capacity - 1));
    for (;;) {
        uint64_t sh; int64_t so, ss;
        read_slot(idx, slot, &sh, &so, &ss);
        if (ss == STATUS_EMPTY) return;
        if (ss == STATUS_OCCUPIED && sh == hash) {
            write_slot(idx, slot, 0, 0, STATUS_TOMBSTONE);
            idx->count--;
            write_header(idx);
            fflush(idx->fp);
            return;
        }
        slot = (slot + 1) & (idx->capacity - 1);
    }
}

/* ── Rebuild (clear all entries) ────────────────────────────────── */

void jade_idx_clear(JadeIndex *idx) {
    if (!idx) return;
    init_file(idx);
}
