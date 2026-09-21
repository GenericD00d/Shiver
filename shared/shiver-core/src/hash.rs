//! Hashes that have to agree with something outside this crate.
//!
//! Nothing here is for hashing in the ordinary sense — Rust's own hashers are better at that and
//! this one is weak. What it is for is producing *the same number* as a different runtime, which is
//! a different requirement and the reason it cannot be `DefaultHasher`.

/// Java's `String.hashCode`.
///
/// Specified rather than merely implemented: `s[0]*31^(n-1) + s[1]*31^(n-2) + … + s[n-1]`, over
/// UTF-16 code units, wrapping like a `jint`. Because it is specified, a Rust caller and a Kotlin
/// caller can both compute it and get the same answer — which is the whole point of it being here.
///
/// Shiver needs that for Android notification ids. The two halves of push identify a server's
/// notification independently: the core posts one while Shiver is running, and `PushReceiver` posts
/// one from a dead process using `token.hashCode()`, where the token is the rail entry id. If those
/// two numbers disagree, a notification posted by one half cannot be taken back by the other, and
/// the shade keeps a notice nothing will ever clear.
///
/// `encode_utf16` and not `chars`: Java hashes UTF-16, so a character outside the basic multilingual
/// plane is two units there and one `char` here. Entry ids are ascii uuids, so the two agree for
/// every input Shiver actually uses — but a function whose entire job is to match Java's answer
/// should match it for the inputs it was not designed around too.
pub fn java_string(text: &str) -> i32 {
    text.encode_utf16().fold(0i32, |hash, unit| {
        hash.wrapping_mul(31).wrapping_add(i32::from(unit))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checked against a real JVM rather than against another implementation of the same guess.
    ///
    /// These numbers came out of `System.out.println(s.hashCode())` on OpenJDK. If this test ever
    /// fails, the thing to fix is this function — the expected values are not ours to choose.
    #[test]
    fn matches_what_a_jvm_answers() {
        for (text, expected) in [
            ("3f2a9c41-7b1e-4d8a-9f33-0c5e6a2b8d17", -891_518_266),
            ("a1b2c3d4-0000-4000-8000-000000000001", -1_340_998_843),
            ("", 0),
            ("a", 97),
            ("shiver", -903_324_049),
            // outside ascii, where a byte-wise or char-wise version would start to disagree
            ("éèê", 231_339),
            // outside the basic multilingual plane: one `char`, two UTF-16 units. This is the case
            // `chars()` gets wrong and `encode_utf16` gets right.
            ("😀", 1_772_899),
        ] {
            assert_eq!(
                java_string(text),
                expected,
                "java_string({text:?}) should match String.hashCode()"
            );
        }
    }

    /// The property the notification ids actually depend on: same input, same number, every time.
    #[test]
    fn is_stable_for_one_input() {
        let id = "3f2a9c41-7b1e-4d8a-9f33-0c5e6a2b8d17";

        assert_eq!(java_string(id), java_string(id));
    }
}
