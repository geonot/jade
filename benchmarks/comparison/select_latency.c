/* R9/B-SELECT — In-process select baseline using two SPSC ring buffers
 * matching benchmarks/select_latency.jade. The previous baseline used
 * poll(2) over pipe(2) which incurs per-event syscalls and is not
 * representative of an in-process select primitive. */

#include <stdio.h>
#include <stdint.h>

#define CAP 16

typedef struct {
    int64_t buf[CAP];
    size_t head;
    size_t tail;
    size_t count;
} ring_t;

static inline int ring_send(ring_t *r, int64_t v) {
    if (r->count == CAP) return 0;
    r->buf[r->tail] = v;
    r->tail = (r->tail + 1) % CAP;
    r->count++;
    return 1;
}

static inline int ring_recv(ring_t *r, int64_t *out) {
    if (r->count == 0) return 0;
    *out = r->buf[r->head];
    r->head = (r->head + 1) % CAP;
    r->count--;
    return 1;
}

int main(void) {
    static ring_t r1, r2;
    long n = 2000000;
    int64_t total = 0;
    for (long i = 0; i < n; i++) {
        ring_send(&r1, i);
        // "select" — try r1, then r2; first ready wins.
        int64_t val;
        if (ring_recv(&r1, &val)) {
            total += val;
        } else if (ring_recv(&r2, &val)) {
            total += val;
        }
    }
    printf("%lld\n", (long long)total);
    return 0;
}
