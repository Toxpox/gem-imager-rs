//! A string that never reaches a log, a `Debug` dump, a snapshot or a crash report.
//!
//! `instruction.md` §10.3 requires secret types to be redacted in `Debug` output. That is not a
//! style preference: the GUI derives `Debug` on its state, `tracing` renders `Debug` for structured
//! fields, and panics print `Debug` for every value in scope. A plain `String` password would leak
//! through all three.

use zeroize::{Zeroize, Zeroizing};

/// A user-supplied secret (account password, Wi-Fi passphrase, VNC password).
///
/// `Debug` prints a fixed placeholder, and the buffer is wiped on drop. Equality is provided
/// because the GUI diffs its customization state between frames; it is a plain byte comparison and
/// is **not** suitable for authentication decisions.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the plaintext.
    ///
    /// Every caller is inside this module tree and feeds it straight into a KDF. Nothing outside
    /// the crate can reach the plaintext.
    pub(super) fn expose(&self) -> &str {
        &self.0
    }

    /// Borrow the plaintext for a masked text field.
    ///
    /// A text input has to be handed the value it is editing, so this one crack in the redaction is
    /// unavoidable. It is named for that single use so that any other call site reads as wrong: the
    /// value must go straight into a `secure(true)` widget and nowhere else — not a log, not a
    /// label, not a clipboard.
    pub fn as_input(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Length in bytes — needed for the UI to show the WPA 8..63 and VNC 8-byte limits without
    /// ever rendering the value itself.
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for Secret {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl std::hash::Hash for Secret {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// A derived secret (crypt hash, PSK hex, VNC ciphertext) that is wiped when it goes out of scope.
pub(super) type DerivedSecret = Zeroizing<String>;

#[cfg(test)]
mod tests {
    use super::*;

    /// The snapshot test from `instruction.md` §10.5: no formatting path may render the plaintext.
    #[test]
    fn debug_output_never_contains_the_plaintext() {
        let s = Secret::new("hunter2-Ağ");
        assert_eq!(format!("{s:?}"), "Secret(<redacted>)");
        assert!(!format!("{s:#?}").contains("hunter2"));

        // The realistic leak is a struct that derives Debug and happens to hold one.
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
    fn length_is_observable_without_exposing_the_value() {
        assert_eq!(Secret::new("12345678").len(), 8);
        assert!(Secret::default().is_empty());
    }
}
