//! Hashes that must match another runtime exactly.

/// Java's `String.hashCode` (UTF-16 units, wrapping `i32`). Android notification ids are computed
/// with this on both sides of push — Rust in the running app and Kotlin in `PushReceiver` — so a
/// notification posted by one can be cancelled by the other.
pub fn java_string(text: &str) -> i32 {
    text.encode_utf16().fold(0i32, |hash, unit| {
        hash.wrapping_mul(31).wrapping_add(i32::from(unit))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Expected values from `String.hashCode()` on OpenJDK.
    #[test]
    fn matches_what_a_jvm_answers() {
        for (text, expected) in [
            ("3f2a9c41-7b1e-4d8a-9f33-0c5e6a2b8d17", -891_518_266),
            ("a1b2c3d4-0000-4000-8000-000000000001", -1_340_998_843),
            ("", 0),
            ("a", 97),
            ("shiver", -903_324_049),
            ("éèê", 231_339),
            ("😀", 1_772_899),
        ] {
            assert_eq!(java_string(text), expected, "java_string({text:?})");
        }
    }
}
