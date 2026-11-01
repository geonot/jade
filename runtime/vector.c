#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <unistd.h>
#include <errno.h>
#include <sys/stat.h>
#include "jinn_rt.h"

#define VEC_MAGIC "JINNVEC\0"
struct JinnVec {
    FILE   *fp;
    int64_t count;
    int64_t dims;
};

JinnVec *jinn_vec_open(const char *path, int64_t dims) {
    jinn_persistence_check_path(path);
    JinnVec *v = calloc(1, sizeof(JinnVec));
    if (!v) jinn_persistence_fail("vector allocation");
    if (dims <= 0 || dims > INT64_MAX / 8) jinn_persistence_fail("vector dimensions");
    v->dims = dims;
    v->fp = fopen(path, "r+b");
    if (v->fp) {
        char magic[8];
        jinn_store_fread(magic, 1, 8, v->fp);
        jinn_store_fread(&v->count, 8, 1, v->fp);
        int64_t stored_dims;
        jinn_store_fread(&stored_dims, 8, 1, v->fp);
        struct stat st;
        if (memcmp(magic, VEC_MAGIC, 8) || stored_dims != dims || v->count < 0 ||
            v->count > (INT64_MAX - 24) / (dims * 8) ||
            fstat(fileno(v->fp), &st) != 0 || st.st_size != 24 + v->count * dims * 8)
            jinn_persistence_fail("vector header validation");
    } else {
        if (errno != ENOENT) jinn_persistence_fail("vector open");
        v->fp = fopen(path, "w+bx");
        if (!v->fp) jinn_persistence_fail("vector creation");
        v->count = 0;
        jinn_store_fwrite(VEC_MAGIC, 1, 8, v->fp);
        jinn_store_fwrite(&v->count, 8, 1, v->fp);
        jinn_store_fwrite(&v->dims, 8, 1, v->fp);
        jinn_store_finish_create(v->fp, path);
    }
    return v;
}
void jinn_vec_close(JinnVec *v) {
    if (!v) return;
    if (v->fp && fclose(v->fp) != 0) jinn_persistence_fail("vector close");
    free(v);
}

static void vec_txn_rollback(void *arg) {
    JinnVec *v = arg;
    if (fseek(v->fp, 8, SEEK_SET) != 0 || fread(&v->count, 8, 1, v->fp) != 1)
        jinn_persistence_fail("vector rollback header");
}
static void vec_txn_guard(JinnVec *v) {
    if (jinn_txn_active()) jinn_txn_track_aux(v->fp, vec_txn_rollback, v);
}
void jinn_vec_insert(JinnVec *v, const double *vec) {
    if (!v || !v->fp) jinn_persistence_fail("vector insert handle");
    vec_txn_guard(v);
    if (v->count >= (INT64_MAX - 24) / (v->dims * 8)) jinn_persistence_fail("vector capacity");
    jinn_store_fseek(v->fp, 0, SEEK_END);
    jinn_store_fwrite(vec, sizeof(double), v->dims, v->fp);
    v->count++;
    jinn_store_fseek(v->fp, 8, SEEK_SET);
    jinn_store_fwrite(&v->count, 8, 1, v->fp);
    jinn_store_fflush(v->fp);
}
int64_t jinn_vec_count(JinnVec *v) {
    return v ? v->count : 0;
}
static double vec_dist_sq(const double *a, const double *b, int64_t dims) {
    double sum = 0.0;
    for (int64_t i = 0; i < dims; i++) {
        double d = a[i] - b[i];
        sum += d * d;
    }
    return sum;
}
int64_t jinn_vec_nearest(JinnVec *v, const double *query, int64_t k,
                         int64_t *out_indices) {
    if (!v || !v->fp || v->count == 0 || k <= 0) return 0;
    if (k > v->count) k = v->count;

    int64_t vec_bytes = v->dims * sizeof(double);
    double *buf = malloc(vec_bytes);
    if (!buf) return 0;
    double  *dists   = malloc(v->count * sizeof(double));
    int64_t *indices = malloc(v->count * sizeof(int64_t));
    if (!dists || !indices) {
        free(buf); free(dists); free(indices);
        return 0;
    }
    jinn_store_fseek(v->fp, 24, SEEK_SET);
    for (int64_t i = 0; i < v->count; i++) {
        jinn_store_fread(buf, sizeof(double), v->dims, v->fp);
        dists[i] = vec_dist_sq(buf, query, v->dims);
        indices[i] = i;
    }
    for (int64_t i = 0; i < k; i++) {
        int64_t min_j = i;
        for (int64_t j = i + 1; j < v->count; j++) {
            if (dists[j] < dists[min_j]) min_j = j;
        }
        if (min_j != i) {
            double td = dists[i]; dists[i] = dists[min_j]; dists[min_j] = td;
            int64_t ti = indices[i]; indices[i] = indices[min_j]; indices[min_j] = ti;
        }
        out_indices[i] = indices[i];
    }
    free(buf);
    free(dists);
    free(indices);
    return k;
}
int64_t jinn_vec_nearest_scored(JinnVec *v, const double *query, int64_t k,
                                int64_t *out_indices, double *out_dists) {
    if (!v || !v->fp || v->count == 0 || k <= 0) return 0;
    if (k > v->count) k = v->count;
    int64_t vec_bytes = v->dims * sizeof(double);
    double *buf = malloc(vec_bytes);
    if (!buf) return 0;

    double  *dists   = malloc(v->count * sizeof(double));
    int64_t *indices = malloc(v->count * sizeof(int64_t));
    if (!dists || !indices) {
        free(buf); free(dists); free(indices);
        return 0;
    }

    jinn_store_fseek(v->fp, 24, SEEK_SET);
    for (int64_t i = 0; i < v->count; i++) {
        jinn_store_fread(buf, sizeof(double), v->dims, v->fp);
        dists[i] = vec_dist_sq(buf, query, v->dims);
        indices[i] = i;
    }
    for (int64_t i = 0; i < k; i++) {
        int64_t min_j = i;
        for (int64_t j = i + 1; j < v->count; j++) {
            if (dists[j] < dists[min_j]) min_j = j;
        }
        if (min_j != i) {
            double td = dists[i]; dists[i] = dists[min_j]; dists[min_j] = td;
            int64_t ti = indices[i]; indices[i] = indices[min_j]; indices[min_j] = ti;
        }
        out_indices[i] = indices[i];
        out_dists[i] = sqrt(dists[i]);
    }
    free(buf);
    free(dists);
    free(indices);
    return k;
}
