/**
 * Jinn @vector store runtime — brute-force nearest-neighbor search.
 *
 * Vectors are stored as contiguous arrays of f64 values.
 * File format: [8B magic "JADEVEC\0"][8B count][8B dims][f64 vectors...]
 * Each vector is dims * 8 bytes.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include "jinn_rt.h"

#define VEC_MAGIC "JADEVEC\0"

struct JinnVec {
    FILE   *fp;
    int64_t count;
    int64_t dims;
};

JinnVec *jinn_vec_open(const char *path, int64_t dims) {
    JinnVec *v = calloc(1, sizeof(JinnVec));
    if (!v) return NULL;
    v->dims = dims;
    v->fp = fopen(path, "r+b");
    if (v->fp) {
        // Read existing header
        char magic[8];
        fread(magic, 1, 8, v->fp);
        fread(&v->count, 8, 1, v->fp);
        int64_t stored_dims;
        fread(&stored_dims, 8, 1, v->fp);
        if (stored_dims != dims) {
            // Dimension mismatch — treat as empty
            v->count = 0;
        }
    } else {
        // Create new file
        v->fp = fopen(path, "w+b");
        if (!v->fp) { free(v); return NULL; }
        v->count = 0;
        fwrite(VEC_MAGIC, 1, 8, v->fp);
        fwrite(&v->count, 8, 1, v->fp);
        fwrite(&v->dims, 8, 1, v->fp);
        fflush(v->fp);
    }
    return v;
}

void jinn_vec_close(JinnVec *v) {
    if (!v) return;
    if (v->fp) fclose(v->fp);
    free(v);
}

void jinn_vec_insert(JinnVec *v, const double *vec) {
    if (!v || !v->fp) return;
    // Seek to end and append vector
    fseek(v->fp, 0, SEEK_END);
    fwrite(vec, sizeof(double), v->dims, v->fp);
    v->count++;
    // Update count in header
    fseek(v->fp, 8, SEEK_SET);
    fwrite(&v->count, 8, 1, v->fp);
    fflush(v->fp);
}

int64_t jinn_vec_count(JinnVec *v) {
    return v ? v->count : 0;
}

/// Compute squared Euclidean distance between two vectors.
static double vec_dist_sq(const double *a, const double *b, int64_t dims) {
    double sum = 0.0;
    for (int64_t i = 0; i < dims; i++) {
        double d = a[i] - b[i];
        sum += d * d;
    }
    return sum;
}

/**
 * Find k nearest neighbors to `query`. Returns count of results written
 * into `out_indices` (up to k). out_indices[i] is the 0-based record index.
 * Uses brute-force linear scan with a simple selection approach.
 */
int64_t jinn_vec_nearest(JinnVec *v, const double *query, int64_t k,
                         int64_t *out_indices) {
    if (!v || !v->fp || v->count == 0 || k <= 0) return 0;
    if (k > v->count) k = v->count;

    int64_t vec_bytes = v->dims * sizeof(double);
    double *buf = malloc(vec_bytes);
    if (!buf) return 0;

    // Allocate distance+index arrays
    double  *dists   = malloc(v->count * sizeof(double));
    int64_t *indices = malloc(v->count * sizeof(int64_t));
    if (!dists || !indices) {
        free(buf); free(dists); free(indices);
        return 0;
    }

    // Seek past header
    fseek(v->fp, 24, SEEK_SET);
    for (int64_t i = 0; i < v->count; i++) {
        fread(buf, sizeof(double), v->dims, v->fp);
        dists[i] = vec_dist_sq(buf, query, v->dims);
        indices[i] = i;
    }

    // Partial sort: find top-k by simple selection
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
