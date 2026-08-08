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
int jinn_dns_resolve(const char *host, char *out_buf, int out_len) {
    struct addrinfo hints, *result;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    if (getaddrinfo(host, NULL, &hints, &result) != 0) return -1;
    int ret = getnameinfo(result->ai_addr, result->ai_addrlen,
                          out_buf, out_len, NULL, 0, NI_NUMERICHOST);
    freeaddrinfo(result);
    return ret == 0 ? 0 : -1;
}
int jinn_dns_resolve_all(const char *host, char *out_buf, int out_len) {
    struct addrinfo hints, *result, *rp;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    if (getaddrinfo(host, NULL, &hints, &result) != 0) return 0;
    int count = 0;
    int pos = 0;
    char addr_str[INET6_ADDRSTRLEN];
    for (rp = result; rp != NULL; rp = rp->ai_next) {
        if (getnameinfo(rp->ai_addr, rp->ai_addrlen,
                        addr_str, sizeof(addr_str), NULL, 0, NI_NUMERICHOST) == 0) {
            int slen = (int)strlen(addr_str);
            if (pos + slen + 1 < out_len) {
                if (pos > 0) { out_buf[pos++] = '\n'; }
                memcpy(out_buf + pos, addr_str, slen);
                pos += slen;
                count++;
            }
        }
    }
    out_buf[pos] = '\0';
    freeaddrinfo(result);
    return count;
}
