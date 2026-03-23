#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <pthread.h>

#define MAILBOX_CAP 256
#define CAP_MASK (MAILBOX_CAP - 1)
#define NUM_WORKERS 8

typedef struct {
    int tag;
    long n;
} msg_t;

typedef struct {
    pthread_mutex_t mutex;
    pthread_cond_t cond_ne;
    pthread_cond_t cond_nf;
    msg_t buf[MAILBOX_CAP];
    long head, tail, count;
    int alive;
    long state_sum;
} mailbox_t;

static void mb_send(mailbox_t *mb, msg_t m) {
    pthread_mutex_lock(&mb->mutex);
    while (mb->count == MAILBOX_CAP)
        pthread_cond_wait(&mb->cond_nf, &mb->mutex);
    mb->buf[mb->tail] = m;
    mb->tail = (mb->tail + 1) & CAP_MASK;
    mb->count++;
    pthread_cond_signal(&mb->cond_ne);
    pthread_mutex_unlock(&mb->mutex);
}

static void *worker_loop(void *arg) {
    mailbox_t *mb = (mailbox_t *)arg;
    while (mb->alive) {
        pthread_mutex_lock(&mb->mutex);
        while (mb->count == 0 && mb->alive)
            pthread_cond_wait(&mb->cond_ne, &mb->mutex);
        if (!mb->alive) {
            pthread_mutex_unlock(&mb->mutex);
            break;
        }
        msg_t m = mb->buf[mb->head];
        mb->head = (mb->head + 1) & CAP_MASK;
        mb->count--;
        pthread_cond_signal(&mb->cond_nf);
        pthread_mutex_unlock(&mb->mutex);
        mb->state_sum += m.n;
    }
    return NULL;
}

static mailbox_t *new_worker(void) {
    mailbox_t *mb = calloc(1, sizeof(mailbox_t));
    mb->alive = 1;
    pthread_mutex_init(&mb->mutex, NULL);
    pthread_cond_init(&mb->cond_ne, NULL);
    pthread_cond_init(&mb->cond_nf, NULL);
    pthread_t tid;
    pthread_create(&tid, NULL, worker_loop, mb);
    pthread_detach(tid);
    return mb;
}

int main(void) {
    mailbox_t *workers[NUM_WORKERS];
    for (int i = 0; i < NUM_WORKERS; i++)
        workers[i] = new_worker();

    msg_t m = {0, 1};
    for (long i = 0; i < 1000000; i++) {
        for (int w = 0; w < NUM_WORKERS; w++)
            mb_send(workers[w], m);
    }
    usleep(800000);
    printf("0\n");
    return 0;
}
