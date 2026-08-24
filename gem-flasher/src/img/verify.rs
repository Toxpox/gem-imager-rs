
use std::io;

use sha2::{Digest as _, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractedIntegrity {
    pub size: u64,
    pub sha256: [u8; 32],
}

impl ExtractedIntegrity {
    pub const fn new(size: u64, sha256: [u8; 32]) -> Self {
        Self { size, sha256 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractGate {
    Declared(ExtractedIntegrity),
    LocalFile,
    UndeclaredLegacyCatalog,
}

impl ExtractGate {
    const fn expectation(self) -> Option<ExtractedIntegrity> {
        match self {
            Self::Declared(integrity) => Some(integrity),
            Self::LocalFile | Self::UndeclaredLegacyCatalog => None,
        }
    }

    pub const fn is_enforced(self) -> bool {
        matches!(self, Self::Declared(_))
    }
}

#[derive(Debug)]
pub(crate) struct ExtractVerifier {
    expected: Option<ExtractedIntegrity>,
    hasher: Sha256,
    seen: u64,
    settled: bool,
}

impl ExtractVerifier {
    pub(crate) fn new(gate: ExtractGate) -> Self {
        Self {
            expected: gate.expectation(),
            hasher: Sha256::new(),
            seen: 0,
            settled: false,
        }
    }

    pub(crate) fn observe(&mut self, chunk: &[u8]) -> io::Result<()> {
        let Some(expected) = self.expected else {
            return Ok(());
        };

        if chunk.is_empty() {
            return self.settle(expected);
        }

        self.seen += chunk.len() as u64;

        if self.seen > expected.size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "image integrity failure: the decoder produced more than the {} bytes the \
                     catalog declares",
                    expected.size
                ),
            ));
        }

        self.hasher.update(chunk);
        Ok(())
    }

    fn settle(&mut self, expected: ExtractedIntegrity) -> io::Result<()> {
        if self.settled {
            return Ok(());
        }
        self.settled = true;

        if self.seen != expected.size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "image integrity failure: the decoder produced {} bytes but the catalog \
                     declares {}",
                    self.seen, expected.size
                ),
            ));
        }

        let actual: [u8; 32] = self
            .hasher
            .clone()
            .finalize()
            .as_slice()
            .try_into()
            .expect("SHA-256 is 32 bytes");

        if actual != expected.sha256 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "image integrity failure: extracted sha256 is {} but the catalog declares {}",
                    const_hex::encode(actual),
                    const_hex::encode(expected.sha256)
                ),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha256(data: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.finalize().into()
    }

    fn declared(payload: &[u8]) -> ExtractVerifier {
        ExtractVerifier::new(ExtractGate::Declared(ExtractedIntegrity::new(
            payload.len() as u64,
            sha256(payload),
        )))
    }

    #[test]
    fn a_matching_stream_passes() {
        let payload = b"exactly these bytes";
        let mut verifier = declared(payload);

        verifier.observe(&payload[..7]).unwrap();
        verifier.observe(&payload[7..]).unwrap();
        verifier.observe(&[]).unwrap();
    }

    #[test]
    fn a_stream_that_ends_early_fails_at_eof() {
        let payload = b"exactly these bytes";
        let mut verifier = declared(payload);

        verifier.observe(&payload[..10]).unwrap();
        let err = verifier.observe(&[]).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("10 bytes"), "{err}");
    }

    #[test]
    fn a_stream_that_runs_long_fails_before_eof() {
        let payload = b"exactly these bytes";
        let mut verifier = declared(payload);

        verifier.observe(payload).unwrap();
        let err = verifier
            .observe(b"and then some more")
            .expect_err("an overlong stream must fail without waiting for EOF");

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_stream_of_the_right_length_with_the_wrong_content_fails() {
        let payload = b"exactly these bytes";
        let mut verifier = declared(payload);

        verifier.observe(b"exactly those bytes").unwrap();
        let err = verifier.observe(&[]).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("sha256"), "{err}");
    }

    #[test]
    fn an_undeclared_gate_verifies_nothing_but_never_reports_success_either() {
        for gate in [ExtractGate::LocalFile, ExtractGate::UndeclaredLegacyCatalog] {
            assert!(!gate.is_enforced());

            let mut verifier = ExtractVerifier::new(gate);
            verifier.observe(b"anything at all").unwrap();
            verifier.observe(&[]).unwrap();
        }
    }

    #[test]
    fn reading_past_eof_does_not_re_run_the_gate() {
        let payload = b"exactly these bytes";
        let mut verifier = declared(payload);

        verifier.observe(payload).unwrap();
        verifier.observe(&[]).unwrap();
        verifier.observe(&[]).unwrap();
    }
}
