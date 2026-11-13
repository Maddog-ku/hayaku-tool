//! Windows DPAPI（目前使用者）加解密，等同 C# `ProtectedData.Protect/Unprotect(data, entropy, DataProtectionScope.CurrentUser)`。
//! 給 auth.bin、eip.bin、slack.bin 用；可以解開 C# 版存的檔案，C# 版也解得開這裡存的。

use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use windows::core::PCWSTR;

fn blob(bytes: &[u8]) -> CRYPT_INTEGER_BLOB {
    // 空陣列也給一個有效的指標（跟 .NET 一樣）
    static EMPTY: [u8; 1] = [0];
    let ptr = if bytes.is_empty() { EMPTY.as_ptr() } else { bytes.as_ptr() };
    CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: ptr as *mut u8 }
}

/// 取出輸出 blob 的內容並釋放。
///
/// SAFETY: `out` 必須是 CryptProtectData／CryptUnprotectData 成功後配置的 blob。
unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    if out.pbData.is_null() {
        return Vec::new();
    }
    // SAFETY: 呼叫端保證 pbData 指向 cbData 個位元組，用完以 LocalFree 釋放
    unsafe {
        let data = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut _)));
        data
    }
}

/// 加密（CurrentUser）。
pub fn protect(data: &[u8], entropy: &[u8]) -> windows::core::Result<Vec<u8>> {
    let input = blob(data);
    let entropy = blob(entropy);
    let mut out = CRYPT_INTEGER_BLOB::default();
    // SAFETY: 輸入 blob 指向呼叫期間有效的記憶體；輸出由系統配置
    unsafe {
        CryptProtectData(&input, PCWSTR::null(), Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)?;
        Ok(take(out))
    }
}

/// 解密（CurrentUser）；別的帳號、別台電腦或 entropy 不同會失敗。
pub fn unprotect(data: &[u8], entropy: &[u8]) -> windows::core::Result<Vec<u8>> {
    let input = blob(data);
    let entropy = blob(entropy);
    let mut out = CRYPT_INTEGER_BLOB::default();
    // SAFETY: 同上
    unsafe {
        CryptUnprotectData(&input, None, Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)?;
        Ok(take(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 加密再解密_內容一樣() {
        let data = "{\"accessToken\": \"中文 token\"}".as_bytes();
        let enc = protect(data, b"HayakuBar.v1").unwrap();
        assert_ne!(data, enc.as_slice());
        assert_eq!(data, unprotect(&enc, b"HayakuBar.v1").unwrap().as_slice());
    }

    #[test]
    fn entropy_不同解不開() {
        let enc = protect(b"secret", b"HayakuBar.v1").unwrap();
        assert!(unprotect(&enc, b"HayakuBar.Eip.v1").is_err());
    }

    #[test]
    fn 空資料也可以() {
        let enc = protect(b"", b"HayakuBar.v1").unwrap();
        assert!(unprotect(&enc, b"HayakuBar.v1").unwrap().is_empty());
    }
}
