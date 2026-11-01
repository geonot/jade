#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <time.h>
#include <unistd.h>
#include <errno.h>
#include <sys/stat.h>
#include "jinn_rt.h"
static const char VER_MAGIC[8] = {'J','I','N','N','V','E','R','\0'};
#define VER_HEADER 8
#define VER_ENTRY_HDR 24
FILE *jinn_ver_open(const char *path) {
    jinn_persistence_check_path(path);
    FILE *f = fopen(path, "r+b");
    if (f) {
        char magic[8];
        if (fread(magic, 1, 8, f) != 8 || memcmp(magic, VER_MAGIC, 8) != 0) {
            jinn_persistence_fail("version header validation");
        }
        return f;
    }
    if (errno != ENOENT) jinn_persistence_fail("version open");
    f = fopen(path, "w+bx");
    if (!f) jinn_persistence_fail("version creation");
    jinn_store_fwrite(VER_MAGIC, 1, 8, f);
    jinn_store_finish_create(f, path);
    return f;
}
void jinn_ver_close(FILE *f) {
    if (f && fclose(f) != 0) jinn_persistence_fail("version close");
}
void jinn_ver_append(FILE *f, int64_t sid, int64_t version,
                     const void *record_data, int64_t rec_size) {
    if (!f || rec_size <= 0) jinn_persistence_fail("version append arguments");
    jinn_txn_track_trunc(f);
    jinn_store_fseek(f, 0, SEEK_END);
    int64_t ts = (int64_t)time(NULL);
    jinn_store_fwrite(&sid, 8, 1, f);
    jinn_store_fwrite(&version, 8, 1, f);
    jinn_store_fwrite(&ts, 8, 1, f);
    jinn_store_fwrite(record_data, (size_t)rec_size, 1, f);
    jinn_store_fflush(f);
}
static int64_t ver_entries(FILE *f, int64_t rec_size) {
    struct stat st;
    if (!f || rec_size <= 0 || rec_size > INT64_MAX - VER_ENTRY_HDR ||
        fflush(f) != 0 || fstat(fileno(f), &st) != 0 || st.st_size < VER_HEADER ||
        (st.st_size - VER_HEADER) % (VER_ENTRY_HDR + rec_size))
        jinn_persistence_fail("version file length");
    jinn_store_fseek(f, VER_HEADER, SEEK_SET);
    return (st.st_size - VER_HEADER) / (VER_ENTRY_HDR + rec_size);
}
int64_t jinn_ver_count(FILE *f, int64_t sid, int64_t rec_size) {
    if (!f) return 0;
    int64_t count = 0;
    int64_t total = ver_entries(f, rec_size);
    for (int64_t i = 0; i < total; i++) {
        int64_t entry_sid;
        jinn_store_fread(&entry_sid, 8, 1, f);
        if (entry_sid == sid) count++;
        jinn_store_fseek(f, 16 + rec_size, SEEK_CUR);
    }
    return count;
}
int64_t jinn_ver_at(FILE *f, int64_t sid, int64_t version,
                    void *out_buf, int64_t rec_size) {
    if (!f) return 0;
    int64_t total = ver_entries(f, rec_size);
    int64_t entry_sid, entry_ver;
    for (int64_t i = 0; i < total; i++) {
        jinn_store_fread(&entry_sid, 8, 1, f);
        jinn_store_fread(&entry_ver, 8, 1, f);
        jinn_store_fseek(f, 8, SEEK_CUR);
        if (entry_sid == sid && entry_ver == version) {
            jinn_store_fread(out_buf, (size_t)rec_size, 1, f);
            return 1;
        }
        jinn_store_fseek(f, rec_size, SEEK_CUR);
    }
    return 0;
}
int64_t jinn_ver_history(FILE *f, int64_t sid,
                         void *out_buf, int64_t rec_size,
                         int64_t max_versions) {
    if (!f) return 0;
    int64_t total = ver_entries(f, rec_size);
    if (max_versions <= 0) return 0;
    if ((uint64_t)max_versions > SIZE_MAX / (uint64_t)rec_size)
        jinn_persistence_fail("version output capacity");
    int64_t written = 0;
    int64_t entry_sid, entry_ver;
    uint8_t *dst = (uint8_t *)out_buf;
    for (int64_t i = 0; i < total && written < max_versions; i++) {
        jinn_store_fread(&entry_sid, 8, 1, f);
        jinn_store_fread(&entry_ver, 8, 1, f);
        jinn_store_fseek(f, 8, SEEK_CUR);
        if (entry_sid == sid) {
            jinn_store_fread(dst + written * rec_size, (size_t)rec_size, 1, f);
            written++;
        } else {
            jinn_store_fseek(f, rec_size, SEEK_CUR);
        }
    }
    return written;
}
typedef struct {
    const uint8_t *entries;
    const uint8_t *keep;
    int64_t        total;
    size_t         entry_size;
} VerImage;
static int ver_fill(FILE *tmp, void *arg) {
    VerImage *im = (VerImage *)arg;
    if (fwrite(VER_MAGIC, 1, 8, tmp) != 8) return -1;
    for (int64_t i = 0; i < im->total; i++) {
        if (im->keep[i] &&
            fwrite(im->entries + (size_t)i * im->entry_size, im->entry_size, 1, tmp) != 1) {
            return -1;
        }
    }
    return 0;
}
void jinn_ver_compact(FILE **fpp, const char *path, int64_t rec_size, int64_t keep_n) {
    if (!fpp || !*fpp || !path || keep_n <= 0) return;
    FILE *f = *fpp;
    int64_t total = ver_entries(f, rec_size);
    if (total == 0) return;
    jinn_txn_track_trunc(f);
    size_t entry_size = VER_ENTRY_HDR + (size_t)rec_size;
    if ((uint64_t)total > SIZE_MAX / entry_size) jinn_persistence_fail("version image capacity");
    uint8_t *entries = (uint8_t *)malloc((size_t)total * entry_size);
    uint8_t *keep = (uint8_t *)calloc((size_t)total, 1);
    if (!entries || !keep) jinn_persistence_fail("version compaction allocation");
    jinn_store_fseek(f, VER_HEADER, SEEK_SET);
    jinn_store_fread(entries, entry_size, (size_t)total, f);
    for (int64_t i = total - 1; i >= 0; i--) {
        int64_t sid_i;
        memcpy(&sid_i, entries + i * entry_size, 8);
        int64_t kept = 0;
        for (int64_t j = total - 1; j >= i; j--) {
            int64_t sid_j;
            memcpy(&sid_j, entries + j * entry_size, 8);
            if (sid_j == sid_i && keep[j]) kept++;
        }
        if (kept < keep_n) keep[i] = 1;
    }
    VerImage im = { entries, keep, total, entry_size };
    if (jinn_atomic_rewrite_reopen(path, ver_fill, &im, fpp) != 0)
        jinn_persistence_fail("version compaction rewrite");
    free(entries);
    free(keep);
}
