/*
 * Standalone Chase-Lev deque stress harness (task 8-14).
 *
 * Drives runtime/deque.c directly from pthreads — no coroutines, no
 * context swaps — so ThreadSanitizer sees only the deque's own memory
 * accesses (the fiber-based scheduler cannot be TSan'd without fiber
 * annotations). Compiled and run by tests/runtime_concurrency.rs under
 * -fsanitize=thread and -fsanitize=address.
 *
 * One owner pushes 5000 synthetic items (forcing the 1024-slot initial
 * buffer through at least two grows) and pops intermittently while four
 * thieves steal concurrently; every item must be consumed exactly once.
 * Before the fix, grow() freed the old buffer under a live thief
 * (ASan: heap-use-after-free) and slots were plain pointers (TSan: data
 * race).
 */
#include "jinn_rt.h"
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#define N_ITEMS 5000
#define N_THIEVES 4

static jinn_deque_t dq;
static _Atomic int seen[N_ITEMS + 1];
static _Atomic int done_flag = 0;
static _Atomic long consumed = 0;

static void consume(jinn_coro_t *c) {
    long v = (long)(intptr_t)c;
    if (v < 1 || v > N_ITEMS) {
        fprintf(stderr, "bogus item %ld\n", v);
        exit(2);
    }
    if (atomic_fetch_add(&seen[v], 1) != 0) {
        fprintf(stderr, "item %ld consumed twice\n", v);
        exit(3);
    }
    atomic_fetch_add(&consumed, 1);
}

static void *thief(void *arg) {
    (void)arg;
    while (!atomic_load(&done_flag)) {
        jinn_coro_t *c = jinn_deque_steal(&dq);
        if (c) consume(c);
    }
    return NULL;
}

int main(void) {
    jinn_deque_init(&dq);
    pthread_t th[N_THIEVES];
    for (int i = 0; i < N_THIEVES; i++) {
        pthread_create(&th[i], NULL, thief, NULL);
    }

    /* Push everything up front so the buffer must grow (1024 -> 2048 ->
     * 4096) while thieves are actively stealing; pop intermittently so the
     * owner side of the protocol is exercised too. */
    for (long v = 1; v <= N_ITEMS; v++) {
        jinn_deque_push(&dq, (jinn_coro_t *)(intptr_t)v);
        if ((v & 255) == 0) {
            jinn_coro_t *c = jinn_deque_pop(&dq);
            if (c) consume(c);
        }
    }
    while (atomic_load(&consumed) < N_ITEMS) {
        jinn_coro_t *c = jinn_deque_pop(&dq);
        if (c) consume(c);
        /* NULL: remaining items are in flight in thieves — keep spinning
         * until the count confirms every item landed exactly once. */
    }
    atomic_store(&done_flag, 1);
    for (int i = 0; i < N_THIEVES; i++) {
        pthread_join(th[i], NULL);
    }
    if (atomic_load(&consumed) != N_ITEMS) {
        fprintf(stderr, "consumed %ld of %d\n", (long)atomic_load(&consumed), N_ITEMS);
        return 4;
    }
    jinn_deque_destroy(&dq);
    printf("ok\n");
    return 0;
}
