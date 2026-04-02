/* Sim for benchmark — C comparison
 * Spawns N threads, each computing fib(25)
 */
#include <stdio.h>
#include <pthread.h>
#include <stdlib.h>

static long fib(long n) {
    if (n < 2) return n;
    return fib(n - 1) + fib(n - 2);
}

static void *worker(void *arg) {
    (void)arg;
    fib(25);
    return NULL;
}

int main(void) {
    int n = 100;
    pthread_t *threads = malloc(sizeof(pthread_t) * (size_t)n);
    for (int i = 0; i < n; i++) {
        pthread_create(&threads[i], NULL, worker, NULL);
    }
    for (int i = 0; i < n; i++) {
        pthread_join(threads[i], NULL);
    }
    free(threads);
    printf("0\n");
    return 0;
}
