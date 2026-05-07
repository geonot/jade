/* runtime/crypto.c — Cryptographic primitives using OpenSSL libcrypto
 *
 * Provides: SHA-256, SHA-512, HMAC-SHA256, AES-256-GCM encrypt/decrypt,
 * secure random bytes. Linked with -lcrypto.
 */
#include <openssl/evp.h>
#include <openssl/hmac.h>
#include <openssl/rand.h>
#include <openssl/err.h>
#include <string.h>
#include <stdlib.h>
#include "jade_rt.h"

/* ── Hashing ─────────────────────────────────────────────── */

/* SHA-256 hash. Writes 32 bytes to out. Returns 0 on success. */
int jade_sha256(const unsigned char *data, long len, unsigned char *out) {
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

/* SHA-512 hash. Writes 64 bytes to out. Returns 0 on success. */
int jade_sha512(const unsigned char *data, long len, unsigned char *out) {
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

/* ── HMAC ────────────────────────────────────────────────── */

/* HMAC-SHA256. Writes 32 bytes to out. Returns 0 on success. */
int jade_hmac_sha256(const unsigned char *key, long key_len,
                     const unsigned char *data, long data_len,
                     unsigned char *out) {
    unsigned int out_len = 32;
    unsigned char *result = HMAC(EVP_sha256(), key, (int)key_len,
                                  data, (size_t)data_len, out, &out_len);
    return result ? 0 : -1;
}

/* ── AES-256-GCM ─────────────────────────────────────────── */

/* AES-256-GCM encrypt.
 * key: 32 bytes, iv: 12 bytes
 * Writes ciphertext to out (same length as plaintext).
 * Writes 16-byte tag to tag_out.
 * Returns ciphertext length on success, -1 on failure. */
long jade_aes_gcm_encrypt(const unsigned char *key, const unsigned char *iv,
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

/* AES-256-GCM decrypt.
 * key: 32 bytes, iv: 12 bytes, tag: 16 bytes
 * Returns plaintext length on success, -1 on failure (including auth failure). */
long jade_aes_gcm_decrypt(const unsigned char *key, const unsigned char *iv,
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
    /* Zero any partially written plaintext before returning failure */
    if (pt_len > 0) OPENSSL_cleanse(out, (size_t)pt_len);
    EVP_CIPHER_CTX_free(ctx);
    return -1;
}

/* ── Random ──────────────────────────────────────────────── */

/* Fill buf with n cryptographically secure random bytes. Returns 0 on success. */
int jade_random_bytes(unsigned char *buf, long n) {
    return RAND_bytes(buf, (int)n) == 1 ? 0 : -1;
}

/* ── Hex encoding (for returning hashes as strings) ──────── */

static const char hex_chars[] = "0123456789abcdef";

/* Encode raw bytes to hex string. out must be at least 2*len+1 bytes. */
void jade_bytes_to_hex(const unsigned char *data, long len, char *out) {
    for (long i = 0; i < len; i++) {
        out[i*2]     = hex_chars[(data[i] >> 4) & 0x0f];
        out[i*2 + 1] = hex_chars[data[i] & 0x0f];
    }
    out[len*2] = '\0';
}
