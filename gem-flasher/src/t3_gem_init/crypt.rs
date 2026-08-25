use cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
use sha_crypt::Sha512Params;

use super::secret::{DerivedSecret, Secret};
use super::{T3GemInitError, WPA_PSK_HEX_LEN};

const CRYPT_SALT_LEN: usize = 16;

const CRYPT_SALT_ALPHABET: &[u8; 64] =
    b"./0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

pub(super) fn sha512_crypt_os_salt(password: &Secret) -> Result<DerivedSecret, T3GemInitError> {
    let salt = random_crypt_salt()?;
    sha512_crypt_with_salt(password, &salt)
}

fn sha512_crypt_with_salt(password: &Secret, salt: &str) -> Result<DerivedSecret, T3GemInitError> {
    let params =
        Sha512Params::new(sha_crypt::ROUNDS_DEFAULT).map_err(|_| T3GemInitError::PasswordHash)?;
    let hash = sha_crypt::sha512_crypt_b64(password.expose().as_bytes(), salt.as_bytes(), &params)
        .map_err(|_| T3GemInitError::PasswordHash)?;

    Ok(DerivedSecret::new(format!("$6${salt}${hash}")))
}

fn random_crypt_salt() -> Result<String, T3GemInitError> {
    let mut bytes = [0u8; CRYPT_SALT_LEN];
    getrandom::fill(&mut bytes).map_err(|_| T3GemInitError::Csprng)?;

    Ok(bytes
        .iter()
        .map(|b| CRYPT_SALT_ALPHABET[usize::from(b & 0x3f)] as char)
        .collect())
}

pub(super) fn normalize_psk_hex(value: &Secret) -> Result<DerivedSecret, T3GemInitError> {
    let raw = value.expose();
    if raw.len() != WPA_PSK_HEX_LEN || !raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(T3GemInitError::WifiPassphraseLength);
    }

    Ok(DerivedSecret::new(raw.to_ascii_lowercase()))
}

pub(super) fn vnc_obfuscate(password: &Secret) -> Result<DerivedSecret, T3GemInitError> {
    const VNC_FIXED_KEY: [u8; 8] = [0x17, 0x52, 0x6b, 0x06, 0x23, 0x4e, 0x58, 0x07];

    let plain = password.expose().as_bytes();
    if plain.len() > super::VNC_PASSWORD_MAX_LEN {
        return Err(T3GemInitError::VncPasswordTooLong { len: plain.len() });
    }

    let mut block = [0u8; 8];
    block[..plain.len()].copy_from_slice(plain);

    let key: [u8; 8] = VNC_FIXED_KEY.map(u8::reverse_bits);
    let cipher = des::Des::new(&GenericArray::from(key));
    let mut block = GenericArray::from(block);
    cipher.encrypt_block(&mut block);

    Ok(DerivedSecret::new(const_hex::encode(block)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha512_crypt_matches_the_reference_vector() {
        let hash = sha512_crypt_with_salt(&Secret::new("Hello world!"), "saltstring").unwrap();
        assert_eq!(
            *hash,
            "$6$saltstring$svn8UoSVapNtMuq1ukKS4tPQd8iKwSMHWjl/O817G3uBnIFNjnQJuesI68u4OTLiBFdcbYEdFCoEOfaS35inz1"
        );

        sha_crypt::sha512_check("Hello world!", &hash).expect("independent verify");
        sha_crypt::sha512_check("Hello world", &hash).expect_err("wrong password must not verify");
    }

    #[test]
    fn sha512_crypt_salt_comes_from_the_os_csprng_and_is_never_reused() {
        let a = sha512_crypt_os_salt(&Secret::new("gemstone")).unwrap();
        let b = sha512_crypt_os_salt(&Secret::new("gemstone")).unwrap();

        assert!(a.starts_with("$6$"));
        assert_ne!(*a, *b);
        sha_crypt::sha512_check("gemstone", &a).expect("verifies against its own salt");

        let salt = &a["$6$".len()..a.len() - 86 - 1];
        assert_eq!(salt.len(), CRYPT_SALT_LEN);
        assert!(salt.bytes().all(|b| CRYPT_SALT_ALPHABET.contains(&b)));
    }

    #[test]
    fn a_64_hex_psk_is_accepted_verbatim_and_anything_else_is_rejected() {
        let psk = "0DC0D6EB90555ED6419756B9A15EC3E3209B63DF707DD508D14581F8982721AF";
        assert_eq!(
            *normalize_psk_hex(&Secret::new(psk)).unwrap(),
            psk.to_ascii_lowercase()
        );

        let too_long = format!("{psk}0");
        let not_hex = "z".repeat(64);
        for bad in [&psk[..63], "0", too_long.as_str(), not_hex.as_str()] {
            assert!(normalize_psk_hex(&Secret::new(bad)).is_err());
        }
    }

    #[test]
    fn vnc_obfuscation_matches_the_cross_checked_vector() {
        assert_eq!(
            *vnc_obfuscate(&Secret::new("1234")).unwrap(),
            "ee5b0e48c8fe9771"
        );
        assert_eq!(
            *vnc_obfuscate(&Secret::new("gemstone")).unwrap(),
            "269936771b41d928"
        );
        assert_eq!(
            *vnc_obfuscate(&Secret::new("1234\0\0\0\0")).unwrap(),
            "ee5b0e48c8fe9771"
        );
    }

    #[test]
    fn the_des_primitive_is_standard_des() {
        use cipher::generic_array::GenericArray;

        let cipher = des::Des::new(&GenericArray::from([0u8; 8]));
        let mut block = GenericArray::from([0u8; 8]);
        cipher.encrypt_block(&mut block);

        assert_eq!(const_hex::encode(block), "8ca64de9c1b123a7");
    }

    #[test]
    fn vnc_password_over_eight_bytes_is_rejected_not_truncated() {
        assert!(matches!(
            vnc_obfuscate(&Secret::new("123456789")),
            Err(T3GemInitError::VncPasswordTooLong { len: 9 })
        ));
        assert!(vnc_obfuscate(&Secret::new("çççç")).is_ok());
        assert!(vnc_obfuscate(&Secret::new("ççççç")).is_err());
    }
}
