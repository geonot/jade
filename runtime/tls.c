/* runtime/tls.c — TLS/SSL wrappers using OpenSSL */
#include <openssl/ssl.h>
#include <openssl/err.h>
#include <openssl/x509.h>
#include <string.h>
#include <unistd.h>
#include <sys/socket.h>
#include <arpa/inet.h>
#include <netdb.h>
#include "jade_rt.h"

static int tls_initialized = 0;

struct jade_tls_conn {
    SSL_CTX *ctx;
    SSL *ssl;
    int fd;
};

void jade_tls_init(void) {
    if (!tls_initialized) {
        SSL_library_init();
        SSL_load_error_strings();
        OpenSSL_add_all_algorithms();
        tls_initialized = 1;
    }
}

/* Create a TLS client connection to host:port.
 * Returns a pointer to jade_tls_conn, or NULL on failure. */
jade_tls_conn *jade_tls_connect(const char *host, int port) {
    jade_tls_init();
    
    const SSL_METHOD *method = TLS_client_method();
    SSL_CTX *ctx = SSL_CTX_new(method);
    if (!ctx) return NULL;
    
    SSL_CTX_set_default_verify_paths(ctx);
    SSL_CTX_set_verify(ctx, SSL_VERIFY_PEER, NULL);
    
    /* DNS resolve + connect */
    struct addrinfo hints, *result;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    
    char port_str[16];
    snprintf(port_str, sizeof(port_str), "%d", port);
    
    if (getaddrinfo(host, port_str, &hints, &result) != 0) {
        SSL_CTX_free(ctx);
        return NULL;
    }
    
    int fd = socket(result->ai_family, result->ai_socktype, result->ai_protocol);
    if (fd < 0) {
        freeaddrinfo(result);
        SSL_CTX_free(ctx);
        return NULL;
    }
    
    if (connect(fd, result->ai_addr, result->ai_addrlen) < 0) {
        freeaddrinfo(result);
        close(fd);
        SSL_CTX_free(ctx);
        return NULL;
    }
    freeaddrinfo(result);
    
    SSL *ssl = SSL_new(ctx);
    SSL_set_fd(ssl, fd);
    SSL_set_tlsext_host_name(ssl, host);
    
    /* Set SNI for certificate verification */
    X509_VERIFY_PARAM *param = SSL_get0_param(ssl);
    X509_VERIFY_PARAM_set1_host(param, host, strlen(host));
    
    if (SSL_connect(ssl) <= 0) {
        SSL_free(ssl);
        close(fd);
        SSL_CTX_free(ctx);
        return NULL;
    }
    
    jade_tls_conn *conn = (jade_tls_conn *)malloc(sizeof(jade_tls_conn));
    conn->ctx = ctx;
    conn->ssl = ssl;
    conn->fd = fd;
    return conn;
}

long jade_tls_send(jade_tls_conn *conn, const char *buf, long len) {
    if (!conn || !conn->ssl) return -1;
    return SSL_write(conn->ssl, buf, (int)len);
}

long jade_tls_recv(jade_tls_conn *conn, char *buf, long len) {
    if (!conn || !conn->ssl) return -1;
    return SSL_read(conn->ssl, buf, (int)len);
}

void jade_tls_close(jade_tls_conn *conn) {
    if (!conn) return;
    if (conn->ssl) {
        SSL_shutdown(conn->ssl);
        SSL_free(conn->ssl);
    }
    if (conn->fd >= 0) close(conn->fd);
    if (conn->ctx) SSL_CTX_free(conn->ctx);
    free(conn);
}

/* DNS resolution: resolve hostname to first IPv4/IPv6 address string.
 * Writes result into out_buf (at most out_len bytes).
 * Returns 0 on success, -1 on failure. */
int jade_dns_resolve(const char *host, char *out_buf, int out_len) {
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

/* DNS resolution: resolve hostname to all addresses.
 * Writes newline-separated IP strings into out_buf.
 * Returns number of addresses found. */
int jade_dns_resolve_all(const char *host, char *out_buf, int out_len) {
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

/* === Server-side TLS, error retrieval, peer info === */

struct jade_tls_listener {
    SSL_CTX *ctx;
    int listen_fd;
};

/* Start a TLS server bound to host:port using cert/key PEM files.
 * Returns a pointer or NULL on failure. */
struct jade_tls_listener *jade_tls_listen(const char *host, int port,
                                          const char *cert_path,
                                          const char *key_path) {
    jade_tls_init();
    const SSL_METHOD *method = TLS_server_method();
    SSL_CTX *ctx = SSL_CTX_new(method);
    if (!ctx) return NULL;
    if (SSL_CTX_use_certificate_file(ctx, cert_path, SSL_FILETYPE_PEM) <= 0 ||
        SSL_CTX_use_PrivateKey_file(ctx, key_path, SSL_FILETYPE_PEM) <= 0) {
        SSL_CTX_free(ctx);
        return NULL;
    }
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) {
        SSL_CTX_free(ctx);
        return NULL;
    }
    int one = 1;
    setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port = htons((unsigned short)port);
    if (host == NULL || strcmp(host, "0.0.0.0") == 0) {
        addr.sin_addr.s_addr = htonl(INADDR_ANY);
    } else {
        if (inet_pton(AF_INET, host, &addr.sin_addr) <= 0) {
            close(fd);
            SSL_CTX_free(ctx);
            return NULL;
        }
    }
    if (bind(fd, (struct sockaddr *)&addr, sizeof(addr)) < 0 ||
        listen(fd, 128) < 0) {
        close(fd);
        SSL_CTX_free(ctx);
        return NULL;
    }
    struct jade_tls_listener *l = (struct jade_tls_listener *)malloc(sizeof(*l));
    l->ctx = ctx;
    l->listen_fd = fd;
    return l;
}

jade_tls_conn *jade_tls_accept(struct jade_tls_listener *l) {
    if (!l) return NULL;
    int cfd = accept(l->listen_fd, NULL, NULL);
    if (cfd < 0) return NULL;
    SSL *ssl = SSL_new(l->ctx);
    SSL_set_fd(ssl, cfd);
    if (SSL_accept(ssl) <= 0) {
        SSL_free(ssl);
        close(cfd);
        return NULL;
    }
    jade_tls_conn *c = (jade_tls_conn *)malloc(sizeof(*c));
    c->ctx = NULL;          /* shared from listener; do not free */
    c->ssl = ssl;
    c->fd = cfd;
    return c;
}

void jade_tls_listener_close(struct jade_tls_listener *l) {
    if (!l) return;
    if (l->listen_fd >= 0) close(l->listen_fd);
    if (l->ctx) SSL_CTX_free(l->ctx);
    free(l);
}

/* Copy the most recent OpenSSL error string into buf. Returns bytes
 * written (excluding NUL), or 0 if no error. */
long jade_tls_last_error(char *buf, long len) {
    unsigned long e = ERR_peek_last_error();
    if (e == 0 || len <= 0) return 0;
    ERR_error_string_n(e, buf, (size_t)len);
    return (long)strlen(buf);
}

/* Copy the peer certificate subject DN into buf. Returns bytes written. */
long jade_tls_peer_cert_subject(jade_tls_conn *conn, char *buf, long len) {
    if (!conn || !conn->ssl) return 0;
    X509 *cert = SSL_get_peer_certificate(conn->ssl);
    if (!cert) return 0;
    X509_NAME *subj = X509_get_subject_name(cert);
    char *line = X509_NAME_oneline(subj, NULL, 0);
    long n = 0;
    if (line) {
        n = (long)strlen(line);
        if (n >= len) n = len - 1;
        if (n > 0) memcpy(buf, line, (size_t)n);
        buf[n] = 0;
        OPENSSL_free(line);
    }
    X509_free(cert);
    return n;
}

/* Copy the negotiated TLS protocol version (e.g. "TLSv1.3"). */
long jade_tls_protocol_version(jade_tls_conn *conn, char *buf, long len) {
    if (!conn || !conn->ssl || len <= 0) return 0;
    const char *v = SSL_get_version(conn->ssl);
    if (!v) return 0;
    long n = (long)strlen(v);
    if (n >= len) n = len - 1;
    memcpy(buf, v, (size_t)n);
    buf[n] = 0;
    return n;
}
