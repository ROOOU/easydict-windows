//! Windows account-bound encryption. Never fall back to plaintext on failure.

#[cfg(windows)]
fn transform(input: &[u8], encrypt: bool) -> std::io::Result<Vec<u8>> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };
    use windows::core::PCWSTR;

    let source = CRYPT_INTEGER_BLOB {
        cbData: input.len().try_into().map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "settings are too large")
        })?,
        pbData: input.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // DPAPI borrows the input and allocates its output with LocalAlloc.
    unsafe {
        let result = if encrypt {
            CryptProtectData(
                &source, PCWSTR::null(), None, None, None,
                CRYPTPROTECT_UI_FORBIDDEN, &mut output,
            )
        } else {
            CryptUnprotectData(
                &source, None, None, None, None,
                CRYPTPROTECT_UI_FORBIDDEN, &mut output,
            )
        };
        result.map_err(|error| {
            std::io::Error::other(format!("Windows DPAPI failed ({})", error.code()))
        })?;
        let bytes = if output.cbData == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec()
        };
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
        Ok(bytes)
    }
}

#[cfg(not(windows))]
fn transform(_input: &[u8], _encrypt: bool) -> std::io::Result<Vec<u8>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Windows DPAPI settings require the original Windows account",
    ))
}

pub(super) fn protect(input: &[u8]) -> std::io::Result<Vec<u8>> {
    transform(input, true)
}

pub(super) fn unprotect(input: &[u8]) -> std::io::Result<Vec<u8>> {
    transform(input, false)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn dpapi_round_trip_and_tamper_rejection() {
        let plain = b"test-only-api-key";
        let encrypted = protect(plain).unwrap();
        assert_ne!(encrypted, plain);
        assert_eq!(unprotect(&encrypted).unwrap(), plain);
        let mut damaged = encrypted;
        let last = damaged.len() - 1;
        damaged[last] ^= 1;
        assert!(unprotect(&damaged).is_err());
        assert!(unprotect(b"not a DPAPI blob").is_err());
    }
}
