#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include "jinn_rt.h"

typedef struct {
    void    *ptr;
    int64_t  len;
    int64_t  cap;
} jinn_vec_header_t;
void *__jinn_vec_slice(void *hdr, int64_t start, int64_t end, int64_t elem_size) {
    jinn_vec_header_t *src = (jinn_vec_header_t *)hdr;
    if (start < 0) start = 0;
    if (end > src->len) end = src->len;
    if (start >= end) {
        jinn_vec_header_t *dst = (jinn_vec_header_t *)malloc(sizeof(jinn_vec_header_t));
        dst->ptr = NULL;
        dst->len = 0;
        dst->cap = 0;
        return dst;
    }
    int64_t new_len = end - start;
    int64_t byte_len = new_len * elem_size;
    void *buf = malloc((size_t)byte_len);
    memcpy(buf, (char *)src->ptr + start * elem_size, (size_t)byte_len);
    jinn_vec_header_t *dst = (jinn_vec_header_t *)malloc(sizeof(jinn_vec_header_t));
    dst->ptr = buf;
    dst->len = new_len;
    dst->cap = new_len;
    return dst;
}
static inline int sso_is_heap(const jinn_sso_t *s) {
    return (s->bytes[23] & 0x80) == 0;
}
static inline const char *sso_data(const jinn_sso_t *s, int64_t *out_len) {
    if (sso_is_heap(s)) {
        const char *ptr;
        int64_t len;
        memcpy(&ptr, s->bytes, 8);
        memcpy(&len, s->bytes + 8, 8);
        *out_len = len;
        return ptr;
    } else {
        int64_t len = (int64_t)((unsigned char)s->bytes[23] & 0x7F);
        if (len > 23) len = 23;
        *out_len = len;
        return s->bytes;
    }
}
static inline jinn_sso_t sso_from_parts(const char *data, int64_t len) {
    jinn_sso_t result;
    memset(&result, 0, 24);
    if (len < 23) {
        memcpy(result.bytes, data, (size_t)len);
        result.bytes[23] = (char)(0x80 | (unsigned char)len);
    } else {
        char *buf = (char *)malloc((size_t)(len + 1));
        memcpy(buf, data, (size_t)len);
        buf[len] = '\0';
        memcpy(result.bytes, &buf, 8);
        memcpy(result.bytes + 8, &len, 8);
        int64_t cap = len + 1;
        memcpy(result.bytes + 16, &cap, 8);
    }
    return result;
}
jinn_sso_t __jinn_str_slice(jinn_sso_t str, int64_t start, int64_t end) {
    int64_t len;
    const char *data = sso_data(&str, &len);
    if (start < 0) start = 0;
    if (end > len) end = len;
    if (start >= end) {
        return sso_from_parts("", 0);
    }
    return sso_from_parts(data + start, end - start);
}
void __jinn_str_clone(jinn_sso_t *out, const jinn_sso_t *src) {
    unsigned char tag = (unsigned char)src->bytes[23];
    if (tag & 0x80) {
        memcpy(out, src, sizeof(*out));
        return;
    }
    char *src_ptr;
    int64_t len;
    int64_t cap;
    memcpy(&src_ptr, src->bytes, 8);
    memcpy(&len, src->bytes + 8, 8);
    memcpy(&cap, src->bytes + 16, 8);
    if (cap <= 0) {
        memcpy(out, src, sizeof(*out));
        return;
    }
    if (src_ptr == NULL) {
        memset(out, 0, 24);
        out->bytes[23] = (char)0x80;
        return;
    }
    memset(out, 0, 24);
    char *dst = (char *)malloc((size_t)cap);
    if (len > 0) {
        memcpy(dst, src_ptr, (size_t)len);
    }
    memcpy(out->bytes, &dst, 8);
    memcpy(out->bytes + 8, &len, 8);
    memcpy(out->bytes + 16, &cap, 8);
}
void *__jinn_vec_clone_pod(void *hdr, int64_t elem_size) {
    if (!hdr) return NULL;
    jinn_vec_header_t *src = (jinn_vec_header_t *)hdr;
    jinn_vec_header_t *dst = (jinn_vec_header_t *)malloc(sizeof(jinn_vec_header_t));
    dst->len = src->len;
    dst->cap = src->cap;
    if (src->cap > 0 && src->ptr) {
        int64_t bytes = src->cap * elem_size;
        dst->ptr = malloc((size_t)bytes);
        if (src->len > 0) {
            memcpy(dst->ptr, src->ptr, (size_t)(src->len * elem_size));
        }
    } else {
        dst->ptr = NULL;
    }
    return dst;
}
typedef struct {
    int64_t *buf;
    int64_t  head;
    int64_t  tail;
    int64_t  cap;
} jinn_udeque_t;
void *__jinn_deque_new(void) {
    jinn_udeque_t *dq = (jinn_udeque_t *)malloc(sizeof(jinn_udeque_t));
    dq->cap = 8;
    dq->buf = (int64_t *)calloc((size_t)dq->cap, sizeof(int64_t));
    dq->head = 0;
    dq->tail = 0;
    return dq;
}
static void udeque_grow(jinn_udeque_t *dq) {
    int64_t old_cap = dq->cap;
    int64_t new_cap = old_cap * 2;
    int64_t *new_buf = (int64_t *)calloc((size_t)new_cap, sizeof(int64_t));
    int64_t n = (dq->tail - dq->head + old_cap) % old_cap;
    for (int64_t i = 0; i < n; i++) {
        new_buf[i] = dq->buf[(dq->head + i) % old_cap];
    }
    free(dq->buf);
    dq->buf = new_buf;
    dq->head = 0;
    dq->tail = n;
    dq->cap = new_cap;
}
void __jinn_deque_push_back(void *handle, int64_t val) {
    jinn_udeque_t *dq = (jinn_udeque_t *)handle;
    if ((dq->tail + 1) % dq->cap == dq->head) udeque_grow(dq);
    dq->buf[dq->tail] = val;
    dq->tail = (dq->tail + 1) % dq->cap;
}
void __jinn_deque_push_front(void *handle, int64_t val) {
    jinn_udeque_t *dq = (jinn_udeque_t *)handle;
    if ((dq->tail + 1) % dq->cap == dq->head) udeque_grow(dq);
    dq->head = (dq->head - 1 + dq->cap) % dq->cap;
    dq->buf[dq->head] = val;
}
int64_t __jinn_deque_pop_front(void *handle) {
    jinn_udeque_t *dq = (jinn_udeque_t *)handle;
    if (dq->head == dq->tail) return 0;
    int64_t val = dq->buf[dq->head];
    dq->head = (dq->head + 1) % dq->cap;
    return val;
}
int64_t __jinn_deque_pop_back(void *handle) {
    jinn_udeque_t *dq = (jinn_udeque_t *)handle;
    if (dq->head == dq->tail) return 0;
    dq->tail = (dq->tail - 1 + dq->cap) % dq->cap;
    return dq->buf[dq->tail];
}
int64_t __jinn_deque_len(void *handle) {
    jinn_udeque_t *dq = (jinn_udeque_t *)handle;
    return (dq->tail - dq->head + dq->cap) % dq->cap;
}

int32_t __jinn_str_cmp(const char *a, int64_t alen, const char *b, int64_t blen) {
    int64_t n = alen < blen ? alen : blen;
    int r = 0;
    if (n > 0 && a && b) r = memcmp(a, b, (size_t)n);
    if (r != 0) return r < 0 ? -1 : 1;
    if (alen == blen) return 0;
    return alen < blen ? -1 : 1;
}
