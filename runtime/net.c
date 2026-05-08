/* runtime/net.c — Thin wrappers for POSIX networking functions
 * whose names collide with Jinn keywords (close, send, delete, etc.)
 */
#include <unistd.h>
#include <sys/socket.h>
#include <arpa/inet.h>
#include <netdb.h>
#include <string.h>
#include "jinn_rt.h"

int jinn_socket(int domain, int type, int protocol) {
    return socket(domain, type, protocol);
}

int jinn_close(int fd) {
    return close(fd);
}

int listen_sock(int fd, int backlog) {
    return listen(fd, backlog);
}

long jinn_send(int fd, const void *buf, long len, int flags) {
    return send(fd, buf, (size_t)len, flags);
}

long jinn_recv(int fd, void *buf, long len, int flags) {
    return recv(fd, buf, (size_t)len, flags);
}

long jinn_sendto(int fd, const void *buf, long len, int flags,
                 const void *addr, int addrlen) {
    return sendto(fd, buf, (size_t)len, flags,
                  (const struct sockaddr *)addr, (socklen_t)addrlen);
}

long jinn_recvfrom(int fd, void *buf, long len, int flags,
                   void *addr, int *addrlen) {
    socklen_t slen = addrlen ? (socklen_t)*addrlen : 0;
    long r = recvfrom(fd, buf, (size_t)len, flags,
                      (struct sockaddr *)addr, addrlen ? &slen : NULL);
    if (addrlen) *addrlen = (int)slen;
    return r;
}
