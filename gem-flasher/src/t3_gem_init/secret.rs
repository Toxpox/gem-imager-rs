//! The workspace `Secret` type, re-exported for this crate's call sites.
//!
//! The implementation moved to `gem-helper` so every crate that handles a user password shares one
//! redacting, zeroizing type (see `gem_helper::secret`). This module keeps the crate-local paths
//! (`super::secret::Secret`, `DerivedSecret`) working unchanged.

pub use gem_helper::secret::{DerivedSecret, Secret};

#[cfg(test)]
mod tests {
    use super::*;

    /// The redaction contract this crate relies on, re-checked at this crate's boundary: the moved
    /// type must still refuse to print the plaintext through any `Debug` path.
    #[test]
    fn debug_output_never_contains_the_plaintext() {
        let s = Secret::new("hunter2-Ağ");
        assert_eq!(format!("{s:?}"), "Secret(<redacted>)");

        #[derive(Debug)]
        struct Holder {
            #[allow(dead_code)]
            password: Secret,
        }
        let rendered = format!("{:?}", Holder { password: s });
        assert!(!rendered.contains("hunter2"));
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn derived_secret_is_a_zeroizing_string() {
        let d = DerivedSecret::new("derived".to_owned());
        assert_eq!(&*d, "derived");
    }
}
