/* runtime/event.c — Event loop using epoll (Linux) or kqueue (macOS/BSD)
 *
 * Provides a multiplexed I/O event loop for non-blocking socket operations.
 * Integrates with the coroutine scheduler: when a socket isn't ready,
 * the coroutine is parked and automatically resumed when the FD becomes ready.
 */
#include "jade_rt.h"
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>

#ifdef __linux__
#include <sys/epoll.h>

/* ── Event loop handle ───────────────────────────────────────── */

typedef struct {
    int epfd;
    int max_events;
} jade_event_loop_t;

/* ── Waiter: associates an fd with a parked coroutine ─────────── */

typedef struct {
    int fd;
    jade_coro_t *coro;   /* parked coroutine to resume */
    int events;           /* EPOLLIN, EPOLLOUT, etc. */
} jade_io_waiter_t;

/* Create a new event loop. Returns handle, or NULL on failure. */
void *jade_event_loop_create(int max_events) {
    if (max_events <= 0) max_events = 256;
    jade_event_loop_t *loop = (jade_event_loop_t *)calloc(1, sizeof(jade_event_loop_t));
    if (!loop) return NULL;
    loop->epfd = epoll_create1(EPOLL_CLOEXEC);
    if (loop->epfd < 0) {
        free(loop);
        return NULL;
    }
    loop->max_events = max_events;
    return loop;
}

/* Destroy an event loop. */
void jade_event_loop_destroy(void *handle) {
    jade_event_loop_t *loop = (jade_event_loop_t *)handle;
    if (!loop) return;
    close(loop->epfd);
    free(loop);
}

/* Set an fd to non-blocking mode. Returns 0 on success. */
int jade_fd_set_nonblock(int fd) {
    int flags = fcntl(fd, F_GETFL, 0);
    if (flags < 0) return -1;
    return fcntl(fd, F_SETFL, flags | O_NONBLOCK);
}

/* Register an fd for read events. waiter_ptr is a pointer to jade_io_waiter_t. */
int jade_event_loop_add_read(void *handle, int fd, void *waiter_ptr) {
    jade_event_loop_t *loop = (jade_event_loop_t *)handle;
    if (!loop) return -1;
    struct epoll_event ev;
    ev.events = EPOLLIN | EPOLLONESHOT;
    ev.data.ptr = waiter_ptr;
    return epoll_ctl(loop->epfd, EPOLL_CTL_ADD, fd, &ev);
}

/* Register an fd for write events. */
int jade_event_loop_add_write(void *handle, int fd, void *waiter_ptr) {
    jade_event_loop_t *loop = (jade_event_loop_t *)handle;
    if (!loop) return -1;
    struct epoll_event ev;
    ev.events = EPOLLOUT | EPOLLONESHOT;
    ev.data.ptr = waiter_ptr;
    return epoll_ctl(loop->epfd, EPOLL_CTL_ADD, fd, &ev);
}

/* Re-arm an fd for read events (after EPOLLONESHOT fires). */
int jade_event_loop_rearm_read(void *handle, int fd, void *waiter_ptr) {
    jade_event_loop_t *loop = (jade_event_loop_t *)handle;
    if (!loop) return -1;
    struct epoll_event ev;
    ev.events = EPOLLIN | EPOLLONESHOT;
    ev.data.ptr = waiter_ptr;
    return epoll_ctl(loop->epfd, EPOLL_CTL_MOD, fd, &ev);
}

/* Re-arm an fd for write events. */
int jade_event_loop_rearm_write(void *handle, int fd, void *waiter_ptr) {
    jade_event_loop_t *loop = (jade_event_loop_t *)handle;
    if (!loop) return -1;
    struct epoll_event ev;
    ev.events = EPOLLOUT | EPOLLONESHOT;
    ev.data.ptr = waiter_ptr;
    return epoll_ctl(loop->epfd, EPOLL_CTL_MOD, fd, &ev);
}

/* Remove an fd from the event loop. */
int jade_event_loop_remove(void *handle, int fd) {
    jade_event_loop_t *loop = (jade_event_loop_t *)handle;
    if (!loop) return -1;
    return epoll_ctl(loop->epfd, EPOLL_CTL_DEL, fd, NULL);
}

/* Poll for ready events. Returns number of events, or -1 on error.
 * timeout_ms: -1 = block indefinitely, 0 = non-blocking, >0 = milliseconds.
 * On return, ready_fds and ready_events arrays are filled with fd/event pairs. */
int jade_event_loop_poll(void *handle, int timeout_ms,
                         int *ready_fds, int *ready_events, int max_ready) {
    jade_event_loop_t *loop = (jade_event_loop_t *)handle;
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
        jade_io_waiter_t *w = (jade_io_waiter_t *)events[i].data.ptr;
        if (w) {
            ready_fds[i] = w->fd;
            ready_events[i] = (int)events[i].events;
            /* Unpark the waiting coroutine if present */
            if (w->coro) {
                jade_sched_unpark(w->coro);
                w->coro = NULL;
            }
        } else {
            ready_fds[i] = -1;
            ready_events[i] = 0;
        }
    }
    return nev;
}

/* ── Simple synchronous poll (no coroutine integration) ────────── */

/* Wait for a single fd to become readable. Returns 0 when ready, -1 on error. */
int jade_event_wait_readable(int fd, int timeout_ms) {
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

/* Wait for a single fd to become writable. */
int jade_event_wait_writable(int fd, int timeout_ms) {
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

/* ── Waiter allocation helpers ────────────────────────────────── */

void *jade_io_waiter_create(int fd) {
    jade_io_waiter_t *w = (jade_io_waiter_t *)calloc(1, sizeof(jade_io_waiter_t));
    if (!w) return NULL;
    w->fd = fd;
    w->coro = NULL;
    w->events = 0;
    return w;
}

void jade_io_waiter_destroy(void *waiter) {
    free(waiter);
}

/* Set the coroutine that should be unparked when this waiter fires. */
void jade_io_waiter_set_coro(void *waiter, void *coro) {
    jade_io_waiter_t *w = (jade_io_waiter_t *)waiter;
    if (w) w->coro = (jade_coro_t *)coro;
}

#else
/* ── Stub for non-Linux (placeholder for future kqueue support) ── */
void *jade_event_loop_create(int max_events) { (void)max_events; return NULL; }
void jade_event_loop_destroy(void *handle) { (void)handle; }
int jade_fd_set_nonblock(int fd) { (void)fd; return -1; }
int jade_event_loop_add_read(void *handle, int fd, void *wp) { (void)handle; (void)fd; (void)wp; return -1; }
int jade_event_loop_add_write(void *handle, int fd, void *wp) { (void)handle; (void)fd; (void)wp; return -1; }
int jade_event_loop_rearm_read(void *handle, int fd, void *wp) { (void)handle; (void)fd; (void)wp; return -1; }
int jade_event_loop_rearm_write(void *handle, int fd, void *wp) { (void)handle; (void)fd; (void)wp; return -1; }
int jade_event_loop_remove(void *handle, int fd) { (void)handle; (void)fd; return -1; }
int jade_event_loop_poll(void *handle, int tms, int *fds, int *evts, int mr) {
    (void)handle; (void)tms; (void)fds; (void)evts; (void)mr; return -1;
}
int jade_event_wait_readable(int fd, int tms) { (void)fd; (void)tms; return -1; }
int jade_event_wait_writable(int fd, int tms) { (void)fd; (void)tms; return -1; }
void *jade_io_waiter_create(int fd) { (void)fd; return NULL; }
void jade_io_waiter_destroy(void *w) { (void)w; }
void jade_io_waiter_set_coro(void *w, void *c) { (void)w; (void)c; }
#endif
