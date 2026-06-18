//! Byte-level ASCII validation helpers.

/// Builds a lookup table marking ASCII alphanumeric bytes and each byte in `extra` as allowed.
const fn ascii_alnum_or_extra_table(extra: &[u8]) -> [bool; 256] {
    let mut table = [false; 256];
    let mut i = 0usize;
    while i < 256 {
        let byte = i as u8;
        table[i] = byte.is_ascii_alphanumeric() || contains_byte(extra, byte);
        i += 1;
    }
    table
}

const fn contains_byte(haystack: &[u8], needle: u8) -> bool {
    let mut i = 0;
    while i < haystack.len() {
        if haystack[i] == needle {
            return true;
        }
        i += 1;
    }
    false
}

/// Returns `true` when every byte in `bytes` is allowed by `allowed`.
#[inline]
pub fn bytes_all_allowed(bytes: &[u8], allowed: &[bool; 256]) -> bool {
    bytes.iter().all(|&b| allowed[b as usize])
}

/// Lookup table for ASCII alnum plus `_`, `:`, `@`, `.`, `/`, and `-`.
pub const ASCII_ALNUM_OR_PATH_EXTRA: [bool; 256] = ascii_alnum_or_extra_table(b"_:@./-");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_all_allowed_accepts_alnum_and_extra() {
        assert!(bytes_all_allowed(
            b"app_state_v1",
            &ASCII_ALNUM_OR_PATH_EXTRA
        ));
        assert!(bytes_all_allowed(
            b"ns/wallet:1",
            &ASCII_ALNUM_OR_PATH_EXTRA
        ));
    }

    #[test]
    fn bytes_all_allowed_rejects_whitespace_and_control() {
        assert!(!bytes_all_allowed(b"a b", &ASCII_ALNUM_OR_PATH_EXTRA));
        assert!(!bytes_all_allowed(b"a\n", &ASCII_ALNUM_OR_PATH_EXTRA));
    }
}
