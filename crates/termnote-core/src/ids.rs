//! Lightweight random ID generation.
//!
//! We intentionally avoid pulling in the `uuid` crate (and its transitive
//! `getrandom` dependency) for what is, in practice, a 128-bit random
//! identifier used purely as an opaque primary key. Reading from
//! `/dev/urandom` directly keeps the dependency graph small and is more than
//! sufficient entropy for session/event identifiers on Linux.

use std::io::Read;

/// Generate a random, lowercase-hex 128-bit identifier, formatted with
/// hyphens in the same visual shape as a UUID (`8-4-4-4-12`) purely for
/// readability; it makes no RFC 4122 version/variant claims.
pub fn new_id() -> String {
    let mut buf = [0u8; 16];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_err()
    {
        // Extremely unlikely fallback: mix the clock and process id so we
        // still produce a usable, if weaker, identifier rather than panic.
        let nanos = crate::time::now_unix_ns() as u128;
        let pid = std::process::id() as u128;
        let mixed = nanos ^ (pid << 64);
        buf.copy_from_slice(&mixed.to_le_bytes());
    }
    let hex: String = buf.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_well_formed() {
        let a = new_id();
        let b = new_id();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        assert_eq!(a.chars().filter(|c| *c == '-').count(), 4);
    }
}
