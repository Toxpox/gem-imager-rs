
pub use gem_helper::secret::{DerivedSecret, Secret};

#[cfg(test)]
mod tests {
    use super::*;

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
