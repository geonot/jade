#include <stdlib.h>
#include <string.h>
#include <stdint.h>

/*
 * Vec header layout (matches codegen):
 *   [0] ptr  — pointer to data buffer
 *   [8] len  — number of elements
 *  [16] cap  — capacity
 */
typedef struct {
    void    *ptr;
    int64_t  len;
    int64_t  cap;
} jade_vec_header_t;

/*
 * String SSO layout (24 bytes):
 *   If tag bit (byte 23, bit 7) is clear → inline: up to 23 chars in-place
 *   If tag bit is set → heap: {ptr, len, cap} packed in 24 bytes
 */

/* __jade_vec_slice(header_ptr, start, end, elem_size) -> new header_ptr */
void *__jade_vec_slice(void *hdr, int64_t start, int64_t end, int64_t elem_size) {
    jade_vec_header_t *src = (jade_vec_header_t *)hdr;
    if (start < 0) start = 0;
    if (end > src->len) end = src->len;
    if (start >= end) {
        /* Empty slice */
        jade_vec_header_t *dst = (jade_vec_header_t *)malloc(sizeof(jade_vec_header_t));
        dst->ptr = NULL;
        dst->len = 0;
        dst->cap = 0;
        return dst;
    }
    int64_t new_len = end - start;
    int64_t byte_len = new_len * elem_size;
    void *buf = malloc((size_t)byte_len);
    memcpy(buf, (char *)src->ptr + start * elem_size, (size_t)byte_len);
    jade_vec_header_t *dst = (jade_vec_header_t *)malloc(sizeof(jade_vec_header_t));
    dst->ptr = buf;
    dst->len = new_len;
    dst->cap = new_len;
    return dst;
}

/*
 * SSO string helpers.
 * Byte 23 bit 7 = tag. 0 → inline, 1 → heap.
 * Inline: data is bytes 0..22, length = 23 - byte[23].
 * Heap:   ptr at offset 0 (8 bytes), len at offset 8 (8 bytes), cap stored with tag.
 */

typedef struct {
    char bytes[24];
} jade_sso_t;

static inline int sso_is_heap(const jade_sso_t *s) {
    return (s->bytes[23] & 0x80) != 0;
}

static inline const char *sso_data(const jade_sso_t *s, int64_t *out_len) {
    if (sso_is_heap(s)) {
        const char *ptr;
        int64_t len;
        memcpy(&ptr, s->bytes, 8);
        memcpy(&len, s->bytes + 8, 8);
        *out_len = len;
        return ptr;
    } else {
        int64_t len = 23 - (int64_t)(unsigned char)s->bytes[23];
        *out_len = len;
        return s->bytes;
    }
}

static inline jade_sso_t sso_from_parts(const char *data, int64_t len) {
    jade_sso_t result;
    memset(&result, 0, 24);
    if (len <= 23) {
        memcpy(result.bytes, data, (size_t)len);
        result.bytes[23] = (char)(23 - len);
    } else {
        char *buf = (char *)malloc((size_t)(len + 1));
        memcpy(buf, data, (size_t)len);
        buf[len] = '\0';
        memcpy(result.bytes, &buf, 8);
        memcpy(result.bytes + 8, &len, 8);
        int64_t cap = len;
        /* Set tag bit */
        cap |= ((int64_t)1 << 63);
        memcpy(result.bytes + 16, &cap, 8);
    }
    return result;
}

/* __jade_str_slice(sso_str, start, end) -> new sso_str */
jade_sso_t __jade_str_slice(jade_sso_t str, int64_t start, int64_t end) {
    int64_t len;
    const char *data = sso_data(&str, &len);
    if (start < 0) start = 0;
    if (end > len) end = len;
    if (start >= end) {
        return sso_from_parts("", 0);
    }
    return sso_from_parts(data + start, end - start);
}

/*
 * User-facing Deque (ring buffer of i64-sized slots).
 * Layout: { int64_t *buf, int64_t head, int64_t tail, int64_t cap }
 */
typedef struct {
    int64_t *buf;
    int64_t  head;
    int64_t  tail;
    int64_t  cap;
} jade_udeque_t;

void *__jade_deque_new(void) {
    jade_udeque_t *dq = (jade_udeque_t *)malloc(sizeof(jade_udeque_t));
    dq->cap = 8;
    dq->buf = (int64_t *)calloc((size_t)dq->cap, sizeof(int64_t));
    dq->head = 0;
    dq->tail = 0;
    return dq;
}

static void udeque_grow(jade_udeque_t *dq) {
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

void __jade_deque_push_back(void *handle, int64_t val) {
    jade_udeque_t *dq = (jade_udeque_t *)handle;
    if ((dq->tail + 1) % dq->cap == dq->head) udeque_grow(dq);
    dq->buf[dq->tail] = val;
    dq->tail = (dq->tail + 1) % dq->cap;
}

void __jade_deque_push_front(void *handle, int64_t val) {
    jade_udeque_t *dq = (jade_udeque_t *)handle;
    if ((dq->tail + 1) % dq->cap == dq->head) udeque_grow(dq);
    dq->head = (dq->head - 1 + dq->cap) % dq->cap;
    dq->buf[dq->head] = val;
}

int64_t __jade_deque_pop_front(void *handle) {
    jade_udeque_t *dq = (jade_udeque_t *)handle;
    if (dq->head == dq->tail) return 0;
    int64_t val = dq->buf[dq->head];
    dq->head = (dq->head + 1) % dq->cap;
    return val;
}

int64_t __jade_deque_pop_back(void *handle) {
    jade_udeque_t *dq = (jade_udeque_t *)handle;
    if (dq->head == dq->tail) return 0;
    dq->tail = (dq->tail - 1 + dq->cap) % dq->cap;
    return dq->buf[dq->tail];
}

int64_t __jade_deque_len(void *handle) {
    jade_udeque_t *dq = (jade_udeque_t *)handle;
    return (dq->tail - dq->head + dq->cap) % dq->cap;
}
