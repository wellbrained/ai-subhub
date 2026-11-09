//! Read-only access to other apps' generic credentials in Windows Credential Manager (the
//! Copilot CLI keeps its GitHub token there, `agy` its Google login). Never writes, never logs a
//! value; callers take only what they need (a token to send to the provider it belongs to, or
//! an email claim).

/// Generic credentials whose target name contains `needle` (case-insensitive), as
/// (target, blob) pairs. Empty off Windows or when nothing matches.
pub fn find_generic(needle: &str) -> Vec<(String, Vec<u8>)> {
    imp::find_generic(&needle.to_lowercase())
}

/// The blob of the generic credential named exactly `target`, if any.
pub fn read_generic(target: &str) -> Option<Vec<u8>> {
    find_generic(target)
        .into_iter()
        .find(|(t, _)| t.eq_ignore_ascii_case(target))
        .map(|(_, b)| b)
}

/// A credential blob as text: UTF-8, or UTF-16LE (how many Windows apps store strings).
pub fn blob_text(blob: &[u8]) -> Option<String> {
    if let Ok(s) = std::str::from_utf8(blob)
        && !s.contains('\0')
    {
        return Some(s.trim().to_string());
    }
    if blob.len().is_multiple_of(2) {
        let wide: Vec<u16> = blob
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        if let Ok(s) = String::from_utf16(&wide) {
            return Some(s.trim_end_matches('\0').trim().to_string());
        }
    }
    None
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Security::Credentials::{
        CRED_TYPE_GENERIC, CREDENTIALW, CredEnumerateW, CredFree,
    };

    pub fn find_generic(needle_lower: &str) -> Vec<(String, Vec<u8>)> {
        let mut out = vec![];
        let mut count: u32 = 0;
        let mut creds: *mut *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: a null filter enumerates all of the user's credentials; on success `creds`
        // points to `count` credential pointers that stay valid until CredFree.
        let ok = unsafe { CredEnumerateW(std::ptr::null(), 0, &mut count, &mut creds) };
        if ok == 0 || creds.is_null() {
            return out;
        }
        for i in 0..count as usize {
            // SAFETY: i < count; each entry is a valid CREDENTIALW until CredFree below.
            let c = unsafe { &**creds.add(i) };
            if c.Type != CRED_TYPE_GENERIC || c.TargetName.is_null() {
                continue;
            }
            // SAFETY: TargetName is a NUL-terminated wide string.
            let target = unsafe { wide_to_string(c.TargetName) };
            if !target.to_lowercase().contains(needle_lower) {
                continue;
            }
            let blob = if c.CredentialBlob.is_null() || c.CredentialBlobSize == 0 {
                vec![]
            } else {
                // SAFETY: the blob has CredentialBlobSize bytes.
                unsafe {
                    std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize)
                        .to_vec()
                }
            };
            out.push((target, blob));
        }
        // SAFETY: frees the buffer CredEnumerateW allocated.
        unsafe { CredFree(creds as *const _) };
        out
    }

    unsafe fn wide_to_string(p: *const u16) -> String {
        let mut len = 0;
        // SAFETY: caller passes a NUL-terminated string.
        while unsafe { *p.add(len) } != 0 {
            len += 1;
        }
        // SAFETY: `len` u16s are readable.
        String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) })
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn find_generic(_needle_lower: &str) -> Vec<(String, Vec<u8>)> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_text_reads_utf8_and_utf16() {
        assert_eq!(blob_text(b"gho_abc").as_deref(), Some("gho_abc"));
        let wide: Vec<u8> = "gho_x".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(blob_text(&wide).as_deref(), Some("gho_x"));
    }
}
