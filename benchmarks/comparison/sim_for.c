/* Sim for benchmark — C comparison
 * Spawns N threads, each computing fib(28 + i % 5)
 */
#include <stdio.h>
#include <pthread.h>
#include <stdlib.h>

static long fib(long n) {
    if (n < 2) return n;
    return fib(n - 1) + fib(n - 2);
}

static void *worker(void *arg) {
    long i = (long)arg;
    long k = 28 + i % 5;
    volatile long result = fib(k);
    (void)result;
    return NULL;
}

int main(void) {
    int n = 1000;
    pthread_t *threads = malloc(sizeof(pthread_t) * (size_t)n);
    for (int i = 0; i < n; i++) {
        pthread_create(&threads[i], NULL, worker, (void *)(long)i);
    }
    for (int i = 0; i < n; i++) {
        pthread_join(threads[i], NULL);
    }
    printf("0\n");
    free(threads);
    return 0;
}
