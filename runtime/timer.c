/*
 * Jinn Runtime — Timer helpers for select timeout.
 */
#include "jinn_rt.h"
#include <time.h>

void jinn_timer_set(jinn_timer_t *t, uint64_t deadline_ns) {
    t->deadline_ns = deadline_ns;
    t->fired = 0;
}

int jinn_timer_check(jinn_timer_t *t) {
    if (t->fired) return 1;
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    uint64_t ns = (uint64_t)now.tv_sec * 1000000000ULL + (uint64_t)now.tv_nsec;
    if (ns >= t->deadline_ns) {
        t->fired = 1;
        return 1;
    }
    return 0;
}

uint64_t jinn_time_now_ns(void) {
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    return (uint64_t)now.tv_sec * 1000000000ULL + (uint64_t)now.tv_nsec;
}
