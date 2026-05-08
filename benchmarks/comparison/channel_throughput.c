/* R9/B-CHANNEL — In-process single-producer/single-consumer ring buffer
 * baseline matching the single-thread ping-pong pattern used by
 * benchmarks/channel_throughput.jn. The previous baseline used
 * pipe(2)+read/write per element which incurs a pair of syscalls per
 * iteration and is not representative of an in-process channel. */

#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>

#define CAP 1024

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
    static ring_t r;
    long n = 1000000;
    int64_t val;
    for (long i = 0; i < n; i++) {
        ring_send(&r, i);
        ring_recv(&r, &val);
    }
    printf("0\n");
    return 0;
}
