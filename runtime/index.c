#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include "jinn_rt.h"
#define IDX_MAGIC     "JINNIDX1"
#define IDX_MAGIC_LEN 8
#define IDX_HEADER    32
#define SLOT_SIZE     24
#define INITIAL_CAP   256
#define STATUS_EMPTY     0
#define STATUS_OCCUPIED  1
#define STATUS_TOMBSTONE 2
uint64_t jinn_idx_hash_i64(int64_t val) {
    return jinn_fnv1a(&val, sizeof(val));
}
uint64_t jinn_idx_hash_str(const char *buf, int64_t len) {
    return jinn_fnv1a(buf, len);
}
uint64_t jinn_idx_hash_f64(double val) {
    return jinn_fnv1a(&val, sizeof(val));
}

struct JinnIndex {
    FILE   *fp;
    int64_t capacity;
    int64_t count;
    int64_t fingerprint;
};

static int read_header(JinnIndex *idx) {
    fseek(idx->fp, 0, SEEK_SET);
    char mag[IDX_MAGIC_LEN];
    if (fread(mag, 1, IDX_MAGIC_LEN, idx->fp) != IDX_MAGIC_LEN) return -1;
    if (memcmp(mag, IDX_MAGIC, IDX_MAGIC_LEN) != 0) return -1;
    if (fread(&idx->capacity, 8, 1, idx->fp) != 1) return -1;
    if (fread(&idx->count, 8, 1, idx->fp) != 1) return -1;
    if (fread(&idx->fingerprint, 8, 1, idx->fp) != 1) return -1;
    return 0;
}

static int write_header(JinnIndex *idx) {
    if (fseek(idx->fp, 0, SEEK_SET) != 0 ||
        fwrite(IDX_MAGIC, 1, IDX_MAGIC_LEN, idx->fp) != IDX_MAGIC_LEN ||
        fwrite(&idx->capacity, 8, 1, idx->fp) != 1 ||
        fwrite(&idx->count, 8, 1, idx->fp) != 1 ||
        fwrite(&idx->fingerprint, 8, 1, idx->fp) != 1) {
        fprintf(stderr, "jinn: index: write_header failed\n");
        return -1;
    }
    return 0;
}
static void init_file(JinnIndex *idx) {
    idx->capacity = INITIAL_CAP;
    idx->count = 0;
    write_header(idx);
    uint8_t zero0[SLOT_SIZE];
    memset(zero0, 0, SLOT_SIZE);
    for (int64_t i = 0; i < INITIAL_CAP; i++) {
        fwrite(zero0, 1, SLOT_SIZE, idx->fp);
    }
    fflush(idx->fp);
}
static int header_is_sane(JinnIndex *idx) {
    int64_t cap = idx->capacity;
    if (cap < INITIAL_CAP) return 0;
    if (cap > ((int64_t)1 << 60)) return 0;
    if ((cap & (cap - 1)) != 0) return 0;
    if (idx->count < 0 || idx->count > cap) return 0;
    if (fseek(idx->fp, 0, SEEK_END) != 0) return 0;
    long end = ftell(idx->fp);
    if (end < (long)(IDX_HEADER + cap * SLOT_SIZE)) return 0;
    return 1;
}
JinnIndex *jinn_idx_open_checked(const char *path, int64_t fingerprint,
                                 int *needs_rebuild) {
    JinnIndex *idx = (JinnIndex *)calloc(1, sizeof(JinnIndex));
    if (!idx) { if (needs_rebuild) *needs_rebuild = 0; return NULL; }
    idx->fp = fopen(path, "r+b");
    if (idx->fp && read_header(idx) == 0 &&
        idx->fingerprint == fingerprint && header_is_sane(idx)) {
        if (needs_rebuild) *needs_rebuild = 0;
        return idx;
    }
    if (idx->fp) fclose(idx->fp);
    idx->fp = fopen(path, "w+b");
    if (!idx->fp) { free(idx); if (needs_rebuild) *needs_rebuild = 0; return NULL; }
    idx->fingerprint = fingerprint;
    init_file(idx);
    if (needs_rebuild) *needs_rebuild = 1;
    return idx;
}
JinnIndex *jinn_idx_open(const char *path) {
    return jinn_idx_open_checked(path, 0, NULL);
}
void jinn_idx_close(JinnIndex *idx) {
    if (!idx) return;
    if (idx->fp) fclose(idx->fp);
    free(idx);
}
static void read_slot(JinnIndex *idx, int64_t slot,
                      uint64_t *hash, int64_t *offset, int64_t *status) {
    *hash = 0;
    *offset = 0;
    *status = 0;
    if (fseek(idx->fp, IDX_HEADER + slot * SLOT_SIZE, SEEK_SET) != 0) return;
    if (fread(hash, 8, 1, idx->fp) != 1 ||
        fread(offset, 8, 1, idx->fp) != 1 ||
        fread(status, 8, 1, idx->fp) != 1) {
        *hash = 0;
        *offset = 0;
        *status = 0;
    }
}
static int write_slot(JinnIndex *idx, int64_t slot,
                      uint64_t hash, int64_t offset, int64_t status) {
    if (fseek(idx->fp, IDX_HEADER + slot * SLOT_SIZE, SEEK_SET) != 0 ||
        fwrite(&hash, 8, 1, idx->fp) != 1 ||
        fwrite(&offset, 8, 1, idx->fp) != 1 ||
        fwrite(&status, 8, 1, idx->fp) != 1) {
        fprintf(stderr, "jinn: index: write_slot failed\n");
        return -1;
    }
    return 0;
}

static void grow(JinnIndex *idx) {
    int64_t old_cap = idx->capacity;

    if (old_cap > ((int64_t)1 << 60)) return;

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
    idx->capacity = old_cap * 2;
    idx->count = 0;
    write_header(idx);

    uint8_t zero[SLOT_SIZE];
    memset(zero, 0, SLOT_SIZE);
    for (int64_t i = 0; i < idx->capacity; i++) {
        fwrite(zero, 1, SLOT_SIZE, idx->fp);
    }
    fflush(idx->fp);

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

static void idx_txn_rollback(void *arg) {
    JinnIndex *idx = (JinnIndex *)arg;
    if (idx && idx->fp) (void)read_header(idx);
}
static void idx_txn_guard(JinnIndex *idx) {
    if (idx && idx->fp && jinn_txn_active()) {
        jinn_txn_track_aux(idx->fp, idx_txn_rollback, idx);
    }
}
void jinn_idx_insert(JinnIndex *idx, uint64_t hash, int64_t record_offset) {
    if (!idx) return;
    idx_txn_guard(idx);
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

int64_t jinn_idx_lookup(JinnIndex *idx, uint64_t hash) {
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
int jinn_idx_contains(JinnIndex *idx, uint64_t hash) {
    return jinn_idx_lookup(idx, hash) >= 0 ? 1 : 0;
}
void jinn_idx_delete(JinnIndex *idx, uint64_t hash) {
    if (!idx) return;
    idx_txn_guard(idx);
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

void jinn_idx_clear(JinnIndex *idx) {
    if (!idx) return;
    init_file(idx);
}
