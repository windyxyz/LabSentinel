//! R2-309 批次 4：Fernet 兼容加解密（与 Python `cryptography.fernet.Fernet` 互操作）。
//!
//! 对齐 `server/app/secretstore.py` 的 POSIX 语义：密钥在 `<data_dir>/secret.key`
//! （32 字节 urlsafe-b64，前 16 字节签名键 + 后 16 字节加密键；不存在则生成）。
//! Windows 上 Python 默认用 DPAPI（本模块不实现 DPAPI）——读到 DPAPI 密文时
//! 解密失败返回 None，调用方按"渠道需重新填写"处理（与 Python unprotect 失败同行为）；
//! Windows 上 Python DPAPI 失败时同样回落本格式，故 Rust 写出的密文 Python 总能解。
//!
//! Fernet 规格（cryptography 实现）：token = base64url( 0x80 ‖ ts_be_u64 ‖ iv(16)
//! ‖ ct )，HMAC-SHA256(sign_key, 0x80‖ts‖iv‖ct) 作为尾段；payload 用
//! AES-128-CBC（enc_key）+ PKCS7，明文前缀 0x80 版本字节。

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use hmac::Mac;
use sha2::Sha256;

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

pub struct Fernet {
    sign_key: [u8; 16],
    enc_key: [u8; 16],
}

fn b64url_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    // 兼容带 padding 的 key 编码
    base64::engine::general_purpose::URL_SAFE
        .decode(s.trim())
        .ok()
        .or_else(|| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(s.trim())
                .ok()
        })
}

fn hmac_sig(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = hmac::Hmac::<Sha256>::new_from_slice(key).expect("HMAC 接受任意长度密钥");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

impl Fernet {
    pub fn from_key_str(b64: &str) -> Option<Self> {
        let raw = b64url_decode(b64)?;
        if raw.len() != 32 {
            return None;
        }
        let mut sign_key = [0u8; 16];
        let mut enc_key = [0u8; 16];
        sign_key.copy_from_slice(&raw[..16]);
        enc_key.copy_from_slice(&raw[16..]);
        Some(Self { sign_key, enc_key })
    }

    /// 读 `<data_dir>/secret.key`；不存在则生成（Python _fernet 同语义，POSIX 0600）。
    pub fn load_or_create(data_dir: &std::path::Path) -> Option<Self> {
        let p = data_dir.join("secret.key");
        let text = match std::fs::read_to_string(&p) {
            Ok(t) if !t.trim().is_empty() => t,
            _ => {
                let mut raw = [0u8; 32];
                getrandom::getrandom(&mut raw).ok()?;
                let key_b64 = {
                    use base64::Engine;
                    base64::engine::general_purpose::URL_SAFE.encode(raw)
                };
                std::fs::write(&p, &key_b64).ok()?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
                }
                key_b64
            }
        };
        Self::from_key_str(&text)
    }

    fn unix_now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> String {
        let ts = Self::unix_now();
        let mut iv = [0u8; 16];
        getrandom::getrandom(&mut iv).expect("system entropy");
        // Fernet 明文 = 0x80 版本字节 + 数据；PKCS7 padding 由 cbc crate 处理
        let mut padded = Vec::with_capacity(plaintext.len() + 17);
        padded.push(0x80u8);
        padded.extend_from_slice(plaintext);
        let ct = Aes128CbcEnc::new((&self.enc_key).into(), (&iv).into())
            .encrypt_padded_vec_mut::<Pkcs7>(&padded);
        let mut head = Vec::with_capacity(1 + 8 + 16);
        head.push(0x80u8);
        head.extend_from_slice(&ts.to_be_bytes());
        head.extend_from_slice(&iv);
        head.extend_from_slice(&ct);
        let sig = hmac_sig(&self.sign_key, &head);
        head.extend_from_slice(&sig);
        b64url_encode(&head)
    }

    /// 解密：失败（DPAPI 密文/篡改/格式错）返回 None——调用方按"渠道需重新填写"处理。
    pub fn decrypt(&self, token: &str) -> Option<Vec<u8>> {
        let raw = b64url_decode(token)?;
        if raw.len() < 1 + 8 + 16 + 32 || raw[0] != 0x80 {
            return None;
        }
        let body_len = raw.len() - 32;
        let (body, sig) = raw.split_at(body_len);
        let mut mac =
            hmac::Hmac::<Sha256>::new_from_slice(&self.sign_key).expect("HMAC 接受任意长度密钥");
        mac.update(body);
        if mac.verify_slice(sig).is_err() {
            return None;
        }
        let ts = u64::from_be_bytes(raw[1..9].try_into().ok()?);
        let now = Self::unix_now();
        // Fernet 语义：拒绝超前时间戳（>60s 时钟偏差）；ttl 不限（Python 默认）
        if ts > now + 60 {
            return None;
        }
        let iv: [u8; 16] = raw[9..25].try_into().ok()?;
        let ct = &raw[25..body_len];
        let padded = Aes128CbcDec::new((&self.enc_key).into(), (&iv).into())
            .decrypt_padded_vec_mut::<Pkcs7>(ct)
            .ok()?;
        if padded.first() != Some(&0x80u8) {
            return None;
        }
        Some(padded[1..].to_vec())
    }
}

/// 与 Python secretstore 同口径的 protect：`enc:v1:` 前缀 + Fernet；加密不可用返回
/// None（调用方拒绝保存，R2-106 拒绝明文语义）。
pub fn protect(value: &str, data_dir: &std::path::Path) -> Option<String> {
    let f = Fernet::load_or_create(data_dir)?;
    Some(format!("enc:v1:{}", f.encrypt(value.as_bytes())))
}

/// 与 Python secretstore 同口径的 unprotect：非 `enc:v1:` 原样返回（历史明文兼容）；
/// 解密失败返回空串（渠道显示"需重新填写"，绝不崩溃）。
pub fn unprotect(value: &str, data_dir: &std::path::Path) -> String {
    if !value.starts_with("enc:v1:") {
        return value.to_string();
    }
    let Some(f) = Fernet::load_or_create(data_dir) else {
        return String::new();
    };
    match f.decrypt(value.trim_start_matches("enc:v1:")) {
        Some(b) => String::from_utf8_lossy(&b).to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let enc = protect(
            "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=abc",
            dir.path(),
        )
        .expect("加密应成功");
        assert!(enc.starts_with("enc:v1:"));
        assert!(!enc.contains("qyapi"));
        let plain = unprotect(&enc, dir.path());
        assert_eq!(
            plain,
            "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=abc"
        );
    }

    #[test]
    fn plaintext_compat() {
        let dir = tempfile::tempdir().unwrap();
        // 历史明文原样返回（Python unprotect 同语义）
        assert_eq!(unprotect("plain-token", dir.path()), "plain-token");
    }

    #[test]
    fn tampered_or_foreign_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let enc = protect("secret", dir.path()).unwrap();
        // 篡改 1 字节 → 解密失败返回空（不崩溃）
        let mut raw = enc.clone().into_bytes();
        let n = raw.len();
        raw[n - 2] ^= 1;
        let tampered = String::from_utf8(raw).unwrap();
        assert_eq!(unprotect(&tampered, dir.path()), "");
        // DPAPI 形态（非本实现产生）→ 失败返回空
        assert_eq!(unprotect("enc:v1:ZGFwYXBpLWRhdGE=", dir.path()), "");
    }

    #[test]
    fn key_persists_across_calls() {
        let dir = tempfile::tempdir().unwrap();
        let e1 = protect("a", dir.path()).unwrap();
        let e2 = protect("b", dir.path()).unwrap();
        assert_eq!(unprotect(&e1, dir.path()), "a");
        assert_eq!(unprotect(&e2, dir.path()), "b");
    }
}
