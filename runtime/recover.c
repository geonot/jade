/*
 * Jinn Store Recovery (task 8-23).
 *
 * Before this file existed the WAL was write-only: nothing ever called
 * jinn_wal_replay, so "the data file is the database and the WAL is a
 * log nobody reads". jinn_store_recover runs at store open (emitted by
 * gen_store_ensure_open): it replays every committed WAL record and
 * upserts it into the data file by `sid`, so a store recovers from a
 * WAL plus a stale — or missing — data file.
 *
 * Every WAL payload is a full record image (insert/update/delete all
 * log the whole record), so replay reduces to upsert-by-sid with
 * later-entries-win; a delete entry whose record was never soft-stamped
 * (the hard-delete path logs the pre-removal image) gets its `deleted`
 * field stamped from the entry timestamp so it stays invisible to
 * queries. If recovery changes the data file, the rewrite goes through
 * the 8-21 atomic temp+rename path, every `.idx` sidecar is unlinked
 * (the next open rebuilds them from the data file — their stored byte
 * offsets are stale), and the WAL is checkpointed: a WAL prefix is
 * discardable only once its effects are durably in the data file.
 */

#define _GNU_SOURCE
#include <dirent.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <unistd.h>
#include "jinn_rt.h"

#define REC_HEADER 40 /* magic 8 + count 8 + rec_size 8 + fingerprint 8 + version 8 */

typedef struct {
    uint8_t *rows;      /* count × rec_size, heap */
    int64_t  count;
    int64_t  cap;
    int64_t  rec_size;
    int64_t  sid_off;
    int64_t  del_off;
    int64_t  changed;
    int64_t  skipped;   /* entries whose payload length != rec_size */
} Recover;

static int64_t rec_sid(const Recover *r, const uint8_t *rec) {
    int64_t sid;
    memcpy(&sid, rec + r->sid_off, 8);
    return sid;
}

static void recover_cb(uint8_t op, const void *payload, uint32_t payload_len,
                       int64_t ts, void *ud) {
    Recover *r = (Recover *)ud;
    if ((int64_t)payload_len != r->rec_size || !payload) {
        r->skipped++;
        return;
    }
    uint8_t rec[8192];
    uint8_t *heap_rec = NULL;
    uint8_t *img = rec;
    if (r->rec_size > (int64_t)sizeof(rec)) {
        heap_rec = (uint8_t *)malloc((size_t)r->rec_size);
        if (!heap_rec) { r->skipped++; return; }
        img = heap_rec;
    }
    memcpy(img, payload, (size_t)r->rec_size);

    /* A delete entry must never revive as a live row: the hard-delete
     * path logs the pre-removal image with `deleted` still zero. */
    if (op == 3 && r->del_off >= 0) {
        int64_t del;
        memcpy(&del, img + r->del_off, 8);
        if (del == 0) memcpy(img + r->del_off, &ts, 8);
    }

    int64_t sid = rec_sid(r, img);
    for (int64_t i = 0; i < r->count; i++) {
        uint8_t *row = r->rows + i * r->rec_size;
        if (rec_sid(r, row) == sid) {
            if (memcmp(row, img, (size_t)r->rec_size) != 0) {
                memcpy(row, img, (size_t)r->rec_size);
                r->changed++;
            }
            free(heap_rec);
            return;
        }
    }
    /* Not present — the crash lost this record from the data file. */
    if (r->count == r->cap) {
        int64_t ncap = r->cap ? r->cap * 2 : 64;
        uint8_t *n = (uint8_t *)realloc(r->rows, (size_t)(ncap * r->rec_size));
        if (!n) { r->skipped++; free(heap_rec); return; }
        r->rows = n;
        r->cap = ncap;
    }
    memcpy(r->rows + r->count * r->rec_size, img, (size_t)r->rec_size);
    r->count++;
    r->changed++;
    free(heap_rec);
}

typedef struct {
    const Recover *r;
    int64_t fingerprint;
    int64_t version;
} RecImage;

static int recover_fill(FILE *tmp, void *arg) {
    RecImage *im = (RecImage *)arg;
    static const char MAGIC[8] = {'J','A','D','E','S','T','R','\0'};
    if (fwrite(MAGIC, 1, 8, tmp) != 8 ||
        fwrite(&im->r->count, 8, 1, tmp) != 1 ||
        fwrite(&im->r->rec_size, 8, 1, tmp) != 1 ||
        fwrite(&im->fingerprint, 8, 1, tmp) != 1 ||
        fwrite(&im->version, 8, 1, tmp) != 1) {
        return -1;
    }
    if (im->r->count > 0 &&
        fwrite(im->r->rows, (size_t)im->r->rec_size, (size_t)im->r->count, tmp)
            != (size_t)im->r->count) {
        return -1;
    }
    return 0;
}

/* Unlink every "{base}.{field}.idx" sidecar next to the store — their
 * stored byte offsets are stale after recovery rewrote the file; the
 * existing open_checked/rebuild path recreates them from the data file. */
static void unlink_indexes(const char *store_path) {
    char dirbuf[4096];
    const char *slash = strrchr(store_path, '/');
    const char *dir = ".";
    const char *base = store_path;
    if (slash) {
        size_t n = (size_t)(slash - store_path);
        if (n == 0 || n >= sizeof(dirbuf)) return;
        memcpy(dirbuf, store_path, n);
        dirbuf[n] = '\0';
        dir = dirbuf;
        base = slash + 1;
    }
    size_t blen = strlen(base);
    if (blen > 6 && strcmp(base + blen - 6, ".store") == 0) blen -= 6;

    DIR *d = opendir(dir);
    if (!d) return;
    struct dirent *e;
    while ((e = readdir(d)) != NULL) {
        size_t nlen = strlen(e->d_name);
        if (nlen > blen + 5 &&
            strncmp(e->d_name, base, blen) == 0 &&
            e->d_name[blen] == '.' &&
            strcmp(e->d_name + nlen - 4, ".idx") == 0) {
            char full[4600];
            snprintf(full, sizeof full, "%s/%s", dir, e->d_name);
            unlink(full);
        }
    }
    closedir(d);
}

/*
 * Recover `store_path` from `wal_path` at open. Returns the number of
 * records repaired/restored, 0 if nothing to do, -1 on error. Called
 * with *store_fpp open on the (possibly just-created) data file.
 * sid_offset < 0 means the store has no `sid` field (@simple) — replay
 * identity is impossible, so recovery is skipped.
 */
int64_t jinn_store_recover(FILE **store_fpp, const char *store_path,
                           const char *wal_path, int64_t rec_size,
                           int64_t sid_offset, int64_t deleted_offset) {
    if (!store_fpp || !*store_fpp || !store_path || !wal_path) return -1;
    if (sid_offset < 0 || rec_size <= 0) return 0;
    if (access(wal_path, F_OK) != 0) return 0; /* no WAL — nothing to replay */

    FILE *wal = jinn_wal_open(wal_path); /* truncates any torn tail, loudly */
    if (!wal) return -1;
    if (jinn_wal_size(wal) == 0) {
        jinn_wal_close(wal);
        return 0;
    }

    FILE *fp = *store_fpp;
    fseek(fp, 8, SEEK_SET);
    int64_t count = 0, file_rec_size = 0, fingerprint = 0, version = 0;
    if (fread(&count, 8, 1, fp) != 1 || fread(&file_rec_size, 8, 1, fp) != 1 ||
        fread(&fingerprint, 8, 1, fp) != 1 || fread(&version, 8, 1, fp) != 1 ||
        count < 0) {
        jinn_wal_close(wal);
        return -1;
    }
    if (file_rec_size != 0 && file_rec_size != rec_size) {
        /* Schema drift between WAL era and file era — migrations own this. */
        jinn_wal_close(wal);
        return 0;
    }

    Recover r = {0};
    r.rec_size = rec_size;
    r.sid_off = sid_offset;
    r.del_off = deleted_offset;
    r.count = count;
    r.cap = count > 0 ? count : 0;
    if (count > 0) {
        r.rows = (uint8_t *)malloc((size_t)(count * rec_size));
        if (!r.rows) {
            jinn_wal_close(wal);
            return -1;
        }
        fseek(fp, REC_HEADER, SEEK_SET);
        if (fread(r.rows, (size_t)rec_size, (size_t)count, fp) != (size_t)count) {
            fprintf(stderr, "jinn: recover: short read of %s\n", store_path);
            free(r.rows);
            jinn_wal_close(wal);
            return -1;
        }
    }

    int64_t replayed = jinn_wal_replay(wal, recover_cb, &r);
    if (r.skipped > 0) {
        fprintf(stderr,
                "jinn: recover: %s: %lld WAL entr%s did not match the record "
                "size and were skipped\n",
                store_path, (long long)r.skipped, r.skipped == 1 ? "y" : "ies");
    }

    if (replayed >= 0 && r.changed > 0) {
        fprintf(stderr,
                "jinn: recover: %s: restoring %lld record%s from the WAL\n",
                store_path, (long long)r.changed, r.changed == 1 ? "" : "s");
        RecImage im = { &r, fingerprint, version };
        if (jinn_atomic_rewrite_reopen(store_path, recover_fill, &im, store_fpp) != 0) {
            free(r.rows);
            jinn_wal_close(wal);
            return -1;
        }
        unlink_indexes(store_path);
    }

    /* The WAL's effects are durably in the data file; the prefix is now
     * discardable — and only now. */
    if (replayed >= 0) jinn_wal_checkpoint(wal);
    jinn_wal_close(wal);
    free(r.rows);
    return r.changed;
}
