#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <time.h>
#include <unistd.h>
#include "jinn_rt.h"
static const char VER_MAGIC[8] = {'J','I','N','N','V','E','R','\0'};
#define VER_HEADER 8
#define VER_ENTRY_HDR 24
FILE *jinn_ver_open(const char *path) {
    FILE *f = fopen(path, "r+b");
    if (f) {
        char magic[8];
        if (fread(magic, 1, 8, f) != 8 || memcmp(magic, VER_MAGIC, 8) != 0) {
            fclose(f);
            return NULL;
        }
        return f;
    }
    f = fopen(path, "w+b");
    if (!f) return NULL;
    fwrite(VER_MAGIC, 1, 8, f);
    fflush(f);
    return f;
}
void jinn_ver_close(FILE *f) {
    if (f) fclose(f);
}
void jinn_ver_append(FILE *f, int64_t sid, int64_t version,
                     const void *record_data, int64_t rec_size) {
    if (!f) return;
    jinn_txn_track_trunc(f);
    fseek(f, 0, SEEK_END);
    int64_t ts = (int64_t)time(NULL);
    fwrite(&sid, 8, 1, f);
    fwrite(&version, 8, 1, f);
    fwrite(&ts, 8, 1, f);
    fwrite(record_data, (size_t)rec_size, 1, f);
    fflush(f);
}
int64_t jinn_ver_count(FILE *f, int64_t sid, int64_t rec_size) {
    if (!f) return 0;
    int64_t count = 0;
    (void)rec_size;
    fseek(f, VER_HEADER, SEEK_SET);
    int64_t entry_sid;
    while (fread(&entry_sid, 8, 1, f) == 1) {
        if (entry_sid == sid) count++;
        fseek(f, 8 + 8 + rec_size, SEEK_CUR);
    }
    return count;
}
int64_t jinn_ver_at(FILE *f, int64_t sid, int64_t version,
                    void *out_buf, int64_t rec_size) {
    if (!f) return 0;
    fseek(f, VER_HEADER, SEEK_SET);
    int64_t entry_sid, entry_ver;
    while (fread(&entry_sid, 8, 1, f) == 1) {
        fread(&entry_ver, 8, 1, f);
        fseek(f, 8, SEEK_CUR);
        if (entry_sid == sid && entry_ver == version) {
            fread(out_buf, (size_t)rec_size, 1, f);
            return 1;
        }
        fseek(f, rec_size, SEEK_CUR);
    }
    return 0;
}
int64_t jinn_ver_history(FILE *f, int64_t sid,
                         void *out_buf, int64_t rec_size,
                         int64_t max_versions) {
    if (!f) return 0;
    fseek(f, VER_HEADER, SEEK_SET);
    int64_t written = 0;
    int64_t entry_sid, entry_ver;
    uint8_t *dst = (uint8_t *)out_buf;
    while (fread(&entry_sid, 8, 1, f) == 1 && written < max_versions) {
        fread(&entry_ver, 8, 1, f);
        fseek(f, 8, SEEK_CUR);
        if (entry_sid == sid) {
            fread(dst + written * rec_size, (size_t)rec_size, 1, f);
            written++;
        } else {
            fseek(f, rec_size, SEEK_CUR);
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
    fseek(f, VER_HEADER, SEEK_SET);
    int64_t total = 0;
    {
        int64_t s;
        while (fread(&s, 8, 1, f) == 1) {
            total++;
            fseek(f, 8 + 8 + rec_size, SEEK_CUR);
        }
    }
    if (total == 0) return;

    size_t entry_size = VER_ENTRY_HDR + (size_t)rec_size;
    uint8_t *entries = (uint8_t *)malloc((size_t)total * entry_size);
    if (!entries) return;
    fseek(f, VER_HEADER, SEEK_SET);
    fread(entries, entry_size, (size_t)total, f);
    uint8_t *keep = (uint8_t *)calloc((size_t)total, 1);
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
    (void)jinn_atomic_rewrite_reopen(path, ver_fill, &im, fpp);
    free(entries);
    free(keep);
}
