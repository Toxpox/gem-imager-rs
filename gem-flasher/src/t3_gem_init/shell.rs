
use super::T3GemInitError;

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
            assert_eq!(quoted, format!("'{payload}'"));
        }
    }

    #[test]
    fn single_quote_is_spliced_not_dropped() {
        assert_eq!(quote("test", "it's").unwrap(), r#"'it'\''s'"#);
        assert_eq!(
            quote("test", "x'; id; echo '").unwrap(),
            r#"'x'\''; id; echo '\'''"#
        );
    }

    #[test]
    fn unicode_passes_through_unchanged() {
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
