//! Sortable, collision-resistant job ids (ULID-shaped, zero dependencies
//! beyond `getrandom`): 48-bit millisecond timestamp + 80-bit OS randomness,
//! Crockford base32, 26 chars.

const CROCKFORD: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Generate a fresh ULID-shaped id.
pub fn new_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);

    let mut rnd = [0u8; 10];
    getrandom::getrandom(&mut rnd).expect("OS random source is unavailable");

    // Layout: [48-bit big-endian timestamp][80-bit randomness] = 128 bits.
    let mut bytes = [0u8; 16];
    let ts_bytes = ((ts_ms & 0xFFFF_FFFF_FFFF) as u64).to_be_bytes();
    bytes[..6].copy_from_slice(&ts_bytes[2..]); // low 48 bits, big-endian
    bytes[6..].copy_from_slice(&rnd);
    let mut value = u128::from_be_bytes(bytes);

    let mut out = [b'0'; 26];
    for i in (0..26).rev() {
        out[i] = CROCKFORD[(value & 0x1F) as usize];
        value >>= 5;
    }
    String::from_utf8(out.to_vec()).expect("crockford output is ASCII")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_are_26_crockford_chars_and_unique() {
        let mut seen = HashSet::new();
        for _ in 0..1000 {
            let id = new_id();
            assert_eq!(id.len(), 26);
            assert!(
                id.bytes().all(|b| CROCKFORD.contains(&b)),
                "bad char in {id}"
            );
            assert!(seen.insert(id), "collision");
        }
    }

    #[test]
    fn ids_sort_by_time() {
        let a = new_id();
        std::thread::sleep(std::time::Duration::from_millis(3));
        let b = new_id();
        assert!(a < b, "{a} should sort before {b}");
    }
}
