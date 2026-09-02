use std::{io, path::Path};

use sha2::{Digest, Sha256};

pub(crate) fn sha256_from_path(p: &Path) -> io::Result<[u8; 32]> {
    let file = std::fs::File::open(p)?;
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];

    loop {
        let read = std::io::Read::read(&mut reader, &mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }

    let hash = hasher
        .finalize()
        .as_slice()
        .try_into()
        .expect("SHA-256 is 32 bytes");

    Ok(hash)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn test_sha256_from_path() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        assert_eq!(
            sha256_from_path(file.path()).unwrap(),
            Sha256::new().finalize().as_slice()
        );

        let data = vec![b'A'; 1000];
        file.write_all(&data).unwrap();
        file.flush().unwrap();

        assert_eq!(
            sha256_from_path(file.path()).unwrap(),
            Sha256::new().chain_update(&data).finalize().as_slice()
        );

        let bad_path = std::path::Path::new("this_file_does_not_exist.txt");
        assert!(sha256_from_path(bad_path).is_err());
    }
}
