#define _GNU_SOURCE
#include <dirent.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <unistd.h>
#include "jinn_rt.h"
#define REC_HEADER 40
typedef struct {
    uint8_t *rows;
    int64_t  count;
    int64_t  cap;
    int64_t  rec_size;
    int64_t  sid_off;
    int64_t  del_off;
    int64_t  changed;
    int64_t  skipped;
    int64_t  oom;
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
        if (!heap_rec) { r->oom++; return; }
        img = heap_rec;
    }
    memcpy(img, payload, (size_t)r->rec_size);
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

    if (r->count == r->cap) {
        int64_t ncap = r->cap ? r->cap * 2 : 64;
        size_t nbytes = jinn_safe_mul(ncap, r->rec_size);
        if (nbytes == 0) { r->oom++; free(heap_rec); return; }
        uint8_t *n = (uint8_t *)realloc(r->rows, nbytes);
        if (!n) { r->oom++; free(heap_rec); return; }
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
int64_t jinn_store_recover(FILE **store_fpp, const char *store_path,
                           const char *wal_path, int64_t rec_size,
                           int64_t sid_offset, int64_t deleted_offset) {
    if (!store_fpp || !*store_fpp || !store_path || !wal_path) return -1;
    if (sid_offset < 0 || rec_size <= 0) return 0;
    if (access(wal_path, F_OK) != 0) return 0;
    FILE *wal = jinn_wal_open(wal_path);
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
        jinn_wal_close(wal);
        return 0;
    }
    if (fseek(fp, 0, SEEK_END) != 0) {
        jinn_wal_close(wal);
        return -1;
    }
    long file_bytes = ftell(fp);
    if (file_bytes < REC_HEADER) {
        fprintf(stderr, "jinn: recover: %s: file is shorter than its header\n", store_path);
        jinn_wal_close(wal);
        return -1;
    }
    int64_t max_records = ((int64_t)file_bytes - REC_HEADER) / rec_size;
    if (count > max_records) {
        fprintf(stderr,
                "jinn: recover: %s: header claims %lld records but the file holds at "
                "most %lld; refusing to read past the end of the file\n",
                store_path, (long long)count, (long long)max_records);
        jinn_wal_close(wal);
        return -1;
    }

    Recover r = {0};
    r.rec_size = rec_size;
    r.sid_off = sid_offset;
    r.del_off = deleted_offset;
    r.count = count;
    r.cap = count > 0 ? count : 0;
    if (count > 0) {
        size_t bytes = jinn_safe_mul(count, rec_size);
        if (bytes == 0) {
            fprintf(stderr, "jinn: recover: %s: record count %lld overflows\n", store_path,
                    (long long)count);
            jinn_wal_close(wal);
            return -1;
        }
        r.rows = (uint8_t *)malloc(bytes);
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
    int complete = jinn_wal_replay_was_complete() && r.oom == 0;
    if (r.skipped > 0) {
        fprintf(stderr,
                "jinn: recover: %s: %lld WAL entr%s did not match the record "
                "size and were skipped\n",
                store_path, (long long)r.skipped, r.skipped == 1 ? "y" : "ies");
    }
    if (!complete) {
        fprintf(stderr,
                "jinn: recover: %s: out of memory during WAL replay — the data "
                "file is left as it was and the WAL is preserved for the next "
                "run\n",
                store_path);
        jinn_wal_close(wal);
        free(r.rows);
        return -1;
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
        jinn_store_drop_indexes(store_path);
    }
    if (replayed >= 0) jinn_wal_checkpoint(wal);
    jinn_wal_close(wal);
    free(r.rows);
    return r.changed;
}
