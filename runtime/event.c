#include "jinn_rt.h"
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#ifdef __linux__
#include <sys/epoll.h>

typedef struct {
    int epfd;
    int max_events;
} jinn_event_loop_t;

typedef struct {
    int fd;
    jinn_coro_t *coro;
    int events;

    _Atomic(int32_t) lock;
    _Atomic(int32_t) fired;
} jinn_io_waiter_t;
static inline void waiter_lock(jinn_io_waiter_t *w) {
    while (atomic_exchange_explicit(&w->lock, 1, memory_order_acquire) != 0) {
#if defined(__x86_64__)
        __builtin_ia32_pause();
#elif defined(__aarch64__)
        __asm__ volatile("yield");
#endif
    }
}
static inline void waiter_unlock(jinn_io_waiter_t *w) {
    atomic_store_explicit(&w->lock, 0, memory_order_release);
}
void *jinn_event_loop_create(int max_events) {
    if (max_events <= 0) max_events = 256;
    jinn_event_loop_t *loop = (jinn_event_loop_t *)calloc(1, sizeof(jinn_event_loop_t));
    if (!loop) return NULL;
    loop->epfd = epoll_create1(EPOLL_CLOEXEC);
    if (loop->epfd < 0) {
        free(loop);
        return NULL;
    }
    loop->max_events = max_events;
    return loop;
}
void jinn_event_loop_destroy(void *handle) {
    jinn_event_loop_t *loop = (jinn_event_loop_t *)handle;
    if (!loop) return;
    close(loop->epfd);
    free(loop);
}

int jinn_fd_set_nonblock(int fd) {
    int flags = fcntl(fd, F_GETFL, 0);
    if (flags < 0) return -1;
    return fcntl(fd, F_SETFL, flags | O_NONBLOCK);
}
int jinn_event_loop_add_read(void *handle, int fd, void *waiter_ptr) {
    jinn_event_loop_t *loop = (jinn_event_loop_t *)handle;
    if (!loop) return -1;
    struct epoll_event ev;
    ev.events = EPOLLIN | EPOLLONESHOT;
    ev.data.ptr = waiter_ptr;
    return epoll_ctl(loop->epfd, EPOLL_CTL_ADD, fd, &ev);
}
int jinn_event_loop_add_write(void *handle, int fd, void *waiter_ptr) {
    jinn_event_loop_t *loop = (jinn_event_loop_t *)handle;
    if (!loop) return -1;
    struct epoll_event ev;
    ev.events = EPOLLOUT | EPOLLONESHOT;
    ev.data.ptr = waiter_ptr;
    return epoll_ctl(loop->epfd, EPOLL_CTL_ADD, fd, &ev);
}
int jinn_event_loop_rearm_read(void *handle, int fd, void *waiter_ptr) {
    jinn_event_loop_t *loop = (jinn_event_loop_t *)handle;
    if (!loop) return -1;
    struct epoll_event ev;
    ev.events = EPOLLIN | EPOLLONESHOT;
    ev.data.ptr = waiter_ptr;
    return epoll_ctl(loop->epfd, EPOLL_CTL_MOD, fd, &ev);
}
int jinn_event_loop_rearm_write(void *handle, int fd, void *waiter_ptr) {
    jinn_event_loop_t *loop = (jinn_event_loop_t *)handle;
    if (!loop) return -1;
    struct epoll_event ev;
    ev.events = EPOLLOUT | EPOLLONESHOT;
    ev.data.ptr = waiter_ptr;
    return epoll_ctl(loop->epfd, EPOLL_CTL_MOD, fd, &ev);
}
int jinn_event_loop_remove(void *handle, int fd) {
    jinn_event_loop_t *loop = (jinn_event_loop_t *)handle;
    if (!loop) return -1;
    return epoll_ctl(loop->epfd, EPOLL_CTL_DEL, fd, NULL);
}
int jinn_event_loop_poll(void *handle, int timeout_ms,
                         int *ready_fds, int *ready_events, int max_ready) {
    jinn_event_loop_t *loop = (jinn_event_loop_t *)handle;
    if (!loop || !ready_fds || !ready_events) return -1;
    int n = (max_ready < loop->max_events) ? max_ready : loop->max_events;
    struct epoll_event *events = (struct epoll_event *)alloca(
        (size_t)n * sizeof(struct epoll_event));
    int nev = epoll_wait(loop->epfd, events, n, timeout_ms);
    if (nev < 0) {
        if (errno == EINTR) return 0;
        return -1;
    }
    for (int i = 0; i < nev; i++) {
        jinn_io_waiter_t *w = (jinn_io_waiter_t *)events[i].data.ptr;
        if (w) {
            ready_fds[i] = w->fd;
            ready_events[i] = (int)events[i].events;
            waiter_lock(w);
            if (w->coro) {
                jinn_coro_t *c = w->coro;
                w->coro = NULL;
                jinn_sched_unpark(c);
            } else {
                atomic_store_explicit(&w->fired, 1, memory_order_release);
            }
            waiter_unlock(w);
        } else {
            ready_fds[i] = -1;
            ready_events[i] = 0;
        }
    }
    return nev;
}
int jinn_event_wait_readable(int fd, int timeout_ms) {
    int epfd = epoll_create1(EPOLL_CLOEXEC);
    if (epfd < 0) return -1;
    struct epoll_event ev;
    ev.events = EPOLLIN;
    ev.data.fd = fd;
    if (epoll_ctl(epfd, EPOLL_CTL_ADD, fd, &ev) < 0) {
        close(epfd);
        return -1;
    }
    struct epoll_event out;
    int nev = epoll_wait(epfd, &out, 1, timeout_ms);
    close(epfd);
    return (nev > 0) ? 0 : -1;
}
int jinn_event_wait_writable(int fd, int timeout_ms) {
    int epfd = epoll_create1(EPOLL_CLOEXEC);
    if (epfd < 0) return -1;
    struct epoll_event ev;
    ev.events = EPOLLOUT;
    ev.data.fd = fd;
    if (epoll_ctl(epfd, EPOLL_CTL_ADD, fd, &ev) < 0) {
        close(epfd);
        return -1;
    }
    struct epoll_event out;
    int nev = epoll_wait(epfd, &out, 1, timeout_ms);
    close(epfd);
    return (nev > 0) ? 0 : -1;
}
void *jinn_io_waiter_create(int fd) {
    jinn_io_waiter_t *w = (jinn_io_waiter_t *)calloc(1, sizeof(jinn_io_waiter_t));
    if (!w) return NULL;
    w->fd = fd;
    w->coro = NULL;
    w->events = 0;
    return w;
}
void jinn_io_waiter_destroy(void *waiter) {
    free(waiter);
}

void jinn_io_waiter_set_coro(void *waiter, void *coro) {
    jinn_io_waiter_t *w = (jinn_io_waiter_t *)waiter;
    if (!w) return;
    waiter_lock(w);
    w->coro = (jinn_coro_t *)coro;
    waiter_unlock(w);
}
void jinn_io_waiter_park(void *waiter) {
    jinn_io_waiter_t *wt = (jinn_io_waiter_t *)waiter;
    if (!wt) return;
    jinn_worker_t *w = jinn_worker_self();
    if (!w || !w->current) {
        while (!atomic_exchange_explicit(&wt->fired, 0, memory_order_acq_rel)) {
            jinn_sched_yield();
        }
        return;
    }
    waiter_lock(wt);
    if (atomic_exchange_explicit(&wt->fired, 0, memory_order_acq_rel)) {
        waiter_unlock(wt);
        return;
    }
    jinn_coro_t *self = w->current;
    wt->coro = self;
    self->state = JINN_CORO_SUSPENDED;
    w->held_lock = &wt->lock;
    w->last_action = SCHED_ACTION_PARK;
    jinn_coro_swap_out(self, &w->sched_ctx);
}
#else
void *jinn_event_loop_create(int max_events) { (void)max_events; return NULL; }
void jinn_event_loop_destroy(void *handle) { (void)handle; }
int jinn_fd_set_nonblock(int fd) { (void)fd; return -1; }
int jinn_event_loop_add_read(void *handle, int fd, void *wp) { (void)handle; (void)fd; (void)wp; return -1; }
int jinn_event_loop_add_write(void *handle, int fd, void *wp) { (void)handle; (void)fd; (void)wp; return -1; }
int jinn_event_loop_rearm_read(void *handle, int fd, void *wp) { (void)handle; (void)fd; (void)wp; return -1; }
int jinn_event_loop_rearm_write(void *handle, int fd, void *wp) { (void)handle; (void)fd; (void)wp; return -1; }
int jinn_event_loop_remove(void *handle, int fd) { (void)handle; (void)fd; return -1; }
int jinn_event_loop_poll(void *handle, int tms, int *fds, int *evts, int mr) {
    (void)handle; (void)tms; (void)fds; (void)evts; (void)mr; return -1;
}
int jinn_event_wait_readable(int fd, int tms) { (void)fd; (void)tms; return -1; }
int jinn_event_wait_writable(int fd, int tms) { (void)fd; (void)tms; return -1; }
void *jinn_io_waiter_create(int fd) { (void)fd; return NULL; }
void jinn_io_waiter_destroy(void *w) { (void)w; }
void jinn_io_waiter_set_coro(void *w, void *c) { (void)w; (void)c; }
void jinn_io_waiter_park(void *w) { (void)w; }
#endif
