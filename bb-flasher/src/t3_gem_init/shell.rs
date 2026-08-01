//! POSIX shell literal quoting — the single place a user-supplied value becomes `config.ini` text.
//!
//! The T3 SDK's `gem-first-boot` **`source`s** `config.ini`, so the file is not data: it is a shell
//! script running as root on first boot. An ordinary INI writer is therefore not safe here. Every
//! value in the file goes through [`quote`], and nothing else in this crate is allowed to build a
//! `key=value` line by concatenation.

use super::T3GemInitError;

/// Wrap a value in a POSIX single-quoted literal.
///
/// Inside single quotes the shell interprets nothing at all, so `$(...)`, backticks, `$VAR`, `"`,
/// `\` and `;` are inert. The one character that cannot appear is `'` itself, which is emitted as
/// the standard `'\''` splice: close the literal, escape a bare quote, reopen it.
///
/// Newlines are *not* handled by quoting. A quoted newline is legal shell, but `gem-first-boot`
/// also greps and `sed`s this file line-by-line, so a value that spans lines would corrupt the
/// file's line structure even though it could never execute. It is rejected instead.
pub(super) fn quote(field: &'static str, value: &str) -> Result<String, T3GemInitError> {
    if let Some(byte) = value.bytes().find(|b| matches!(b, b'\0' | b'\r' | b'\n')) {
        return Err(T3GemInitError::ControlCharacter { field, byte });
    }

    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for c in value.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The injection payloads from `instruction.md` §10.5. None of them may leave the literal.
    #[test]
    fn shell_metacharacters_stay_inside_the_literal() {
        for payload in [
            "$(id)",
            "`id`",
            "$USER",
            "a; rm -rf /",
            "a && reboot",
            "a|b",
            "a>/etc/passwd",
            "back\\slash",
            "double\"quote",
            "${IFS}",
            "*",
            "~root",
            "#comment",
        ] {
            let quoted = quote("test", payload).unwrap();
            assert!(quoted.starts_with('\'') && quoted.ends_with('\''));
            // No unescaped quote can appear in the middle, which is what would end the literal
            // early and let the rest of the payload be parsed as code.
            assert_eq!(quoted, format!("'{payload}'"));
        }
    }

    #[test]
    fn single_quote_is_spliced_not_dropped() {
        assert_eq!(quote("test", "it's").unwrap(), r#"'it'\''s'"#);
        // The classic break-out attempt: close the quote and append a command.
        assert_eq!(
            quote("test", "x'; id; echo '").unwrap(),
            r#"'x'\''; id; echo '\'''"#
        );
    }

    #[test]
    fn unicode_passes_through_unchanged() {
        // Turkish SSIDs and hostnames are a first-class case, not an edge case.
        assert_eq!(quote("test", "Ağ-Çekirdek").unwrap(), "'Ağ-Çekirdek'");
    }

    #[test]
    fn control_characters_are_rejected() {
        for bad in ["a\nb", "a\rb", "a\0b", "\n"] {
            assert!(matches!(
                quote("test", bad),
                Err(T3GemInitError::ControlCharacter { .. })
            ));
        }
    }

    #[test]
    fn empty_value_is_a_valid_empty_literal() {
        assert_eq!(quote("test", "").unwrap(), "''");
    }
}
