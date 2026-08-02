#include <openssl/evp.h>
#include <openssl/hmac.h>
#include <openssl/rand.h>
#include <openssl/err.h>
#include <openssl/kdf.h>
#include <openssl/core_names.h>
#include <openssl/params.h>
#include <openssl/opensslv.h>
#include <stdint.h>
#include <time.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include "jinn_rt.h"
int jinn_sha256(const unsigned char *data, long len, unsigned char *out) {
    EVP_MD_CTX *ctx = EVP_MD_CTX_new();
    if (!ctx) return -1;
    if (EVP_DigestInit_ex(ctx, EVP_sha256(), NULL) != 1 ||
        EVP_DigestUpdate(ctx, data, (size_t)len) != 1 ||
        EVP_DigestFinal_ex(ctx, out, NULL) != 1) {
        EVP_MD_CTX_free(ctx);
        return -1;
    }
    EVP_MD_CTX_free(ctx);
    return 0;
}
int jinn_sha512(const unsigned char *data, long len, unsigned char *out) {
    EVP_MD_CTX *ctx = EVP_MD_CTX_new();
    if (!ctx) return -1;
    if (EVP_DigestInit_ex(ctx, EVP_sha512(), NULL) != 1 ||
        EVP_DigestUpdate(ctx, data, (size_t)len) != 1 ||
        EVP_DigestFinal_ex(ctx, out, NULL) != 1) {
        EVP_MD_CTX_free(ctx);
        return -1;
    }
    EVP_MD_CTX_free(ctx);
    return 0;
}
int jinn_hmac_sha256(const unsigned char *key, long key_len,
                     const unsigned char *data, long data_len,
                     unsigned char *out) {
    unsigned int out_len = 32;
    unsigned char *result = HMAC(EVP_sha256(), key, (int)key_len,
                                  data, (size_t)data_len, out, &out_len);
    return result ? 0 : -1;
}
long jinn_aes_gcm_encrypt(const unsigned char *key, const unsigned char *iv,
                           const unsigned char *plaintext, long pt_len,
                           const unsigned char *aad, long aad_len,
                           unsigned char *out, unsigned char *tag_out) {
    EVP_CIPHER_CTX *ctx = EVP_CIPHER_CTX_new();
    if (!ctx) return -1;
    int len = 0;
    long ct_len = 0;
    if (EVP_EncryptInit_ex(ctx, EVP_aes_256_gcm(), NULL, NULL, NULL) != 1) goto fail;
    if (EVP_CIPHER_CTX_ctrl(ctx, EVP_CTRL_GCM_SET_IVLEN, 12, NULL) != 1) goto fail;
    if (EVP_EncryptInit_ex(ctx, NULL, NULL, key, iv) != 1) goto fail;
    if (aad && aad_len > 0) {
        if (EVP_EncryptUpdate(ctx, NULL, &len, aad, (int)aad_len) != 1) goto fail;
    }

    if (EVP_EncryptUpdate(ctx, out, &len, plaintext, (int)pt_len) != 1) goto fail;
    ct_len = len;
    if (EVP_EncryptFinal_ex(ctx, out + len, &len) != 1) goto fail;
    ct_len += len;
    if (EVP_CIPHER_CTX_ctrl(ctx, EVP_CTRL_GCM_GET_TAG, 16, tag_out) != 1) goto fail;

    EVP_CIPHER_CTX_free(ctx);
    return ct_len;

fail:
    EVP_CIPHER_CTX_free(ctx);
    return -1;
}

long jinn_aes_gcm_decrypt(const unsigned char *key, const unsigned char *iv,
                           const unsigned char *ciphertext, long ct_len,
                           const unsigned char *aad, long aad_len,
                           const unsigned char *tag,
                           unsigned char *out) {
    EVP_CIPHER_CTX *ctx = EVP_CIPHER_CTX_new();
    if (!ctx) return -1;
    int len = 0;
    long pt_len = 0;
    if (EVP_DecryptInit_ex(ctx, EVP_aes_256_gcm(), NULL, NULL, NULL) != 1) goto fail;
    if (EVP_CIPHER_CTX_ctrl(ctx, EVP_CTRL_GCM_SET_IVLEN, 12, NULL) != 1) goto fail;
    if (EVP_DecryptInit_ex(ctx, NULL, NULL, key, iv) != 1) goto fail;
    if (aad && aad_len > 0) {
        if (EVP_DecryptUpdate(ctx, NULL, &len, aad, (int)aad_len) != 1) goto fail;
    }
    if (EVP_DecryptUpdate(ctx, out, &len, ciphertext, (int)ct_len) != 1) goto fail;
    pt_len = len;
    if (EVP_CIPHER_CTX_ctrl(ctx, EVP_CTRL_GCM_SET_TAG, 16, (void *)tag) != 1) goto fail;
    if (EVP_DecryptFinal_ex(ctx, out + len, &len) != 1) goto fail;
    pt_len += len;
    EVP_CIPHER_CTX_free(ctx);
    return pt_len;
fail:

    if (pt_len > 0) OPENSSL_cleanse(out, (size_t)pt_len);
    EVP_CIPHER_CTX_free(ctx);
    return -1;
}


int jinn_random_bytes(unsigned char *buf, long n) {
    return RAND_bytes(buf, (int)n) == 1 ? 0 : -1;
}
static const char hex_chars[] = "0123456789abcdef";

void jinn_bytes_to_hex(const unsigned char *data, long len, char *out) {
    for (long i = 0; i < len; i++) {
        out[i*2]     = hex_chars[(data[i] >> 4) & 0x0f];
        out[i*2 + 1] = hex_chars[data[i] & 0x0f];
    }
    out[len*2] = '\0';
}

long jinn_evp_digest(const char *alg, const unsigned char *data, long len,
                     unsigned char *out) {
    const EVP_MD *md = EVP_get_digestbyname(alg);
    if (!md) return -1;
    EVP_MD_CTX *ctx = EVP_MD_CTX_new();
    if (!ctx) return -1;
    unsigned int n = 0;
    long rc = -1;
    if (EVP_DigestInit_ex(ctx, md, NULL) != 1) goto done;
    if (EVP_DigestUpdate(ctx, data, (size_t)len) != 1) goto done;
    if (EVP_DigestFinal_ex(ctx, out, &n) != 1) goto done;
    rc = (long)n;
done:
    EVP_MD_CTX_free(ctx);
    return rc;
}
long jinn_evp_hmac(const char *alg, const unsigned char *key, long key_len,
                   const unsigned char *data, long data_len,
                   unsigned char *out) {
    const EVP_MD *md = EVP_get_digestbyname(alg);
    if (!md) return -1;
    unsigned int n = 0;
    if (!HMAC(md, key, (int)key_len, data, (size_t)data_len, out, &n))
        return -1;
    return (long)n;
}

int jinn_pbkdf2(const char *alg, const unsigned char *pass, long pass_len,
                const unsigned char *salt, long salt_len, long iters,
                long dklen, unsigned char *out) {
    const EVP_MD *md = EVP_get_digestbyname(alg);
    if (!md) return -1;
    return PKCS5_PBKDF2_HMAC((const char *)pass, (int)pass_len, salt,
                             (int)salt_len, (int)iters, md, (int)dklen, out)
               == 1
               ? 0
               : -1;
}

long jinn_aes_cbc_encrypt(const unsigned char *key, const unsigned char *iv,
                          const unsigned char *pt, long pt_len,
                          unsigned char *out) {
    EVP_CIPHER_CTX *ctx = EVP_CIPHER_CTX_new();
    if (!ctx) return -1;
    int len = 0;
    long total = -1;
    if (EVP_EncryptInit_ex(ctx, EVP_aes_256_cbc(), NULL, key, iv) != 1) goto done;
    if (EVP_EncryptUpdate(ctx, out, &len, pt, (int)pt_len) != 1) goto done;
    total = len;
    if (EVP_EncryptFinal_ex(ctx, out + total, &len) != 1) { total = -1; goto done; }
    total += len;
done:
    EVP_CIPHER_CTX_free(ctx);
    return total;
}
long jinn_aes_cbc_decrypt(const unsigned char *key, const unsigned char *iv,
                          const unsigned char *ct, long ct_len,
                          unsigned char *out) {
    EVP_CIPHER_CTX *ctx = EVP_CIPHER_CTX_new();
    if (!ctx) return -1;
    int len = 0;
    long total = -1;
    if (EVP_DecryptInit_ex(ctx, EVP_aes_256_cbc(), NULL, key, iv) != 1) goto done;
    if (EVP_DecryptUpdate(ctx, out, &len, ct, (int)ct_len) != 1) goto done;
    total = len;
    if (EVP_DecryptFinal_ex(ctx, out + total, &len) != 1) { total = -1; goto done; }
    total += len;
done:
    EVP_CIPHER_CTX_free(ctx);
    return total;
}
long jinn_chacha20_poly1305_encrypt(const unsigned char *key, const unsigned char *nonce,
                                    const unsigned char *pt, long pt_len,
                                    const unsigned char *aad, long aad_len,
                                    unsigned char *out, unsigned char *tag) {
    EVP_CIPHER_CTX *ctx = EVP_CIPHER_CTX_new();
    if (!ctx) return -1;
    int len = 0;
    long total = -1;
    if (EVP_EncryptInit_ex(ctx, EVP_chacha20_poly1305(), NULL, NULL, NULL) != 1) goto done;
    if (EVP_CIPHER_CTX_ctrl(ctx, EVP_CTRL_AEAD_SET_IVLEN, 12, NULL) != 1) goto done;
    if (EVP_EncryptInit_ex(ctx, NULL, NULL, key, nonce) != 1) goto done;
    if (aad_len > 0 && EVP_EncryptUpdate(ctx, NULL, &len, aad, (int)aad_len) != 1) goto done;
    if (EVP_EncryptUpdate(ctx, out, &len, pt, (int)pt_len) != 1) goto done;
    total = len;
    if (EVP_EncryptFinal_ex(ctx, out + total, &len) != 1) { total = -1; goto done; }
    total += len;
    if (EVP_CIPHER_CTX_ctrl(ctx, EVP_CTRL_AEAD_GET_TAG, 16, tag) != 1) total = -1;
done:
    EVP_CIPHER_CTX_free(ctx);
    return total;
}
long jinn_chacha20_poly1305_decrypt(const unsigned char *key, const unsigned char *nonce,
                                    const unsigned char *ct, long ct_len,
                                    const unsigned char *aad, long aad_len,
                                    const unsigned char *tag, unsigned char *out) {
    EVP_CIPHER_CTX *ctx = EVP_CIPHER_CTX_new();
    if (!ctx) return -1;
    int len = 0;
    long total = -1;
    if (EVP_DecryptInit_ex(ctx, EVP_chacha20_poly1305(), NULL, NULL, NULL) != 1) goto done;
    if (EVP_CIPHER_CTX_ctrl(ctx, EVP_CTRL_AEAD_SET_IVLEN, 12, NULL) != 1) goto done;
    if (EVP_DecryptInit_ex(ctx, NULL, NULL, key, nonce) != 1) goto done;
    if (aad_len > 0 && EVP_DecryptUpdate(ctx, NULL, &len, aad, (int)aad_len) != 1) goto done;
    if (EVP_DecryptUpdate(ctx, out, &len, ct, (int)ct_len) != 1) goto done;
    total = len;
    if (EVP_CIPHER_CTX_ctrl(ctx, EVP_CTRL_AEAD_SET_TAG, 16, (void *)tag) != 1) goto done;
    if (EVP_DecryptFinal_ex(ctx, out + total, &len) != 1) { total = -1; goto done; }
    total += len;
done:
    EVP_CIPHER_CTX_free(ctx);
    return total;
}
int jinn_argon2id(const unsigned char *pass, long pass_len,
                  const unsigned char *salt, long salt_len,
                  long t_cost, long m_cost_kib, long parallelism,
                  long dklen, unsigned char *out) {
#if OPENSSL_VERSION_NUMBER >= 0x30200000L
    EVP_KDF *kdf = EVP_KDF_fetch(NULL, "ARGON2ID", NULL);
    if (!kdf) return -1;
    EVP_KDF_CTX *ctx = EVP_KDF_CTX_new(kdf);
    EVP_KDF_free(kdf);
    if (!ctx) return -1;
    OSSL_PARAM params[7];
    int i = 0;
    uint32_t lanes = (uint32_t)parallelism;
    uint32_t mem = (uint32_t)m_cost_kib;
    uint32_t iter = (uint32_t)t_cost;
    params[i++] = OSSL_PARAM_construct_octet_string("pass", (void *)pass, (size_t)pass_len);
    params[i++] = OSSL_PARAM_construct_octet_string("salt", (void *)salt, (size_t)salt_len);
    params[i++] = OSSL_PARAM_construct_uint32("iter", &iter);
    params[i++] = OSSL_PARAM_construct_uint32("memcost", &mem);
    params[i++] = OSSL_PARAM_construct_uint32("lanes", &lanes);
    params[i++] = OSSL_PARAM_construct_uint32("threads", &lanes);
    params[i++] = OSSL_PARAM_construct_end();
    int rc = EVP_KDF_derive(ctx, out, (size_t)dklen, params) == 1 ? 0 : -1;
    EVP_KDF_CTX_free(ctx);
    return rc;
#else
    (void)pass; (void)pass_len; (void)salt; (void)salt_len;
    (void)t_cost; (void)m_cost_kib; (void)parallelism; (void)dklen; (void)out;
    return -1;
#endif
}
int jinn_scrypt(const unsigned char *pass, long pass_len,
                const unsigned char *salt, long salt_len,
                long n, long r, long p, long dklen, unsigned char *out) {
    return EVP_PBE_scrypt((const char *)pass, (size_t)pass_len, salt,
                          (size_t)salt_len, (uint64_t)n, (uint64_t)r,
                          (uint64_t)p, 0, out, (size_t)dklen) == 1
               ? 0
               : -1;
}
long jinn_hex_to_bytes(const char *hex, long hex_len, unsigned char *out) {
    if (hex_len % 2 != 0) return -1;
    long bytes = hex_len / 2;
    for (long i = 0; i < bytes; i++) {
        unsigned int b;
        char tmp[3] = { hex[i*2], hex[i*2+1], 0 };
        if (sscanf(tmp, "%02x", &b) != 1) return -1;
        out[i] = (unsigned char)b;
    }
    return bytes;
}
