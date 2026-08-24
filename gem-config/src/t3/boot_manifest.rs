
use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;

use crate::t3::sha256::Sha256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootArtifact {
    pub name: String,
    pub sha256: Sha256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedBootManifest {
    artifacts: Vec<BootArtifact>,
}

impl VerifiedBootManifest {
    pub fn artifacts(&self) -> &[BootArtifact] {
        &self.artifacts
    }

    pub fn artifact(&self, name: &str) -> Option<&BootArtifact> {
        self.artifacts.iter().find(|a| a.name == name)
    }

    pub fn from_stored(
        stored: impl IntoIterator<Item = BootArtifact>,
        required: &[&str],
    ) -> Result<Self, BootManifestError> {
        let by_name: BTreeMap<String, BootArtifact> = stored
            .into_iter()
            .map(|artifact| (artifact.name.clone(), artifact))
            .collect();

        collect_required(by_name, required)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootManifestError {
    Malformed(String),
    MissingArtifact(String),
    InvalidHash {
        artifact: String,
        reason: String,
    },
    ConflictingArtifact(String),
}

impl fmt::Display for BootManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(reason) => write!(f, "boot manifest is malformed: {reason}"),
            Self::MissingArtifact(name) => {
                write!(
                    f,
                    "boot manifest does not publish `{name}`; DFU cannot start"
                )
            }
            Self::InvalidHash { artifact, reason } => {
                write!(
                    f,
                    "boot manifest hash for `{artifact}` is unusable: {reason}"
                )
            }
            Self::ConflictingArtifact(name) => {
                write!(f, "boot manifest publishes `{name}` more than once")
            }
        }
    }
}

impl std::error::Error for BootManifestError {}

#[derive(Debug, Deserialize)]
struct RawManifest {
    files: Vec<RawArtifact>,
}

#[derive(Debug, Deserialize)]
struct RawArtifact {
    name: String,
    sha256: String,
}

pub fn parse_boot_manifest(
    body: &[u8],
    required: &[&str],
) -> Result<VerifiedBootManifest, BootManifestError> {
    let raw: RawManifest = serde_json::from_slice(body)
        .map_err(|err| BootManifestError::Malformed(err.to_string()))?;

    let mut by_name: BTreeMap<String, BootArtifact> = BTreeMap::new();

    for artifact in raw.files {
        let sha256 =
            Sha256::parse(&artifact.sha256).map_err(|err| BootManifestError::InvalidHash {
                artifact: artifact.name.clone(),
                reason: err.to_string(),
            })?;

        let parsed = BootArtifact {
            name: artifact.name,
            sha256,
        };

        if by_name.contains_key(&parsed.name) {
            return Err(BootManifestError::ConflictingArtifact(parsed.name));
        }

        by_name.insert(parsed.name.clone(), parsed);
    }

    collect_required(by_name, required)
}

fn collect_required(
    mut by_name: BTreeMap<String, BootArtifact>,
    required: &[&str],
) -> Result<VerifiedBootManifest, BootManifestError> {
    let artifacts = required
        .iter()
        .map(|name| {
            by_name
                .remove(*name)
                .ok_or_else(|| BootManifestError::MissingArtifact((*name).to_owned()))
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(VerifiedBootManifest { artifacts })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::t3::canonical::DfuProfile;

    const LIVE_MANIFEST: &[u8] = include_bytes!("../../tests/fixtures/t3/boot_manifest.json");

    fn with_required<R>(f: impl FnOnce(&[&str]) -> R) -> R {
        let profile = DfuProfile::t3_gem_o1();
        f(&profile.required_artifacts())
    }

    #[test]
    fn the_live_manifest_publishes_every_required_stage() {
        let manifest = with_required(|req| parse_boot_manifest(LIVE_MANIFEST, req)).unwrap();

        assert_eq!(manifest.artifacts().len(), 3);
        assert_eq!(manifest.artifacts()[0].name, "tiboot3.bin");
        assert_eq!(manifest.artifacts()[1].name, "tispl.bin");
        assert_eq!(manifest.artifacts()[2].name, "u-boot.img");
        assert!(manifest.artifact("tiboot3.bin").is_some());
        assert!(manifest.artifact("nonsense.bin").is_none());
    }

    #[test]
    fn a_manifest_missing_a_stage_is_refused() {
        let body = br#"{"files":[
            {"name":"tiboot3.bin","sha256":"b677db13afd0b4104fdbd17281c20fec234fbb4d6908647e3a940ea07a0c2acc"},
            {"name":"tispl.bin","sha256":"cb0cdea0bd4c6eb430905702069b74719a5081755ea1a3e93b0dac526c16fd0e"}
        ]}"#;

        let err = with_required(|req| parse_boot_manifest(body, req)).unwrap_err();
        assert_eq!(
            err,
            BootManifestError::MissingArtifact("u-boot.img".to_owned())
        );
    }

    #[test]
    fn an_empty_manifest_is_not_a_success() {
        let err = with_required(|req| parse_boot_manifest(br#"{"files":[]}"#, req)).unwrap_err();
        assert!(matches!(err, BootManifestError::MissingArtifact(_)));
    }

    #[test]
    fn a_malformed_document_is_refused() {
        assert!(matches!(
            with_required(|req| parse_boot_manifest(b"not json at all", req)),
            Err(BootManifestError::Malformed(_))
        ));
        assert!(matches!(
            with_required(|req| parse_boot_manifest(b"<html><body>404</body></html>", req)),
            Err(BootManifestError::Malformed(_))
        ));
    }

    #[test]
    fn an_unusable_hash_is_refused_rather_than_skipped() {
        let body = br#"{"files":[
            {"name":"tiboot3.bin","sha256":"not-a-hash"},
            {"name":"tispl.bin","sha256":"cb0cdea0bd4c6eb430905702069b74719a5081755ea1a3e93b0dac526c16fd0e"},
            {"name":"u-boot.img","sha256":"96414fabe62bc3c15d98b78534e03c631b2a2f4a0d7541e435832c1dcb82c6d3"}
        ]}"#;

        assert!(matches!(
            with_required(|req| parse_boot_manifest(body, req)),
            Err(BootManifestError::InvalidHash { .. })
        ));
    }

    #[test]
    fn an_extra_future_artifact_does_not_disable_a_working_board() {
        let body = br#"{"files":[
            {"name":"tiboot3.bin","sha256":"b677db13afd0b4104fdbd17281c20fec234fbb4d6908647e3a940ea07a0c2acc"},
            {"name":"tispl.bin","sha256":"cb0cdea0bd4c6eb430905702069b74719a5081755ea1a3e93b0dac526c16fd0e"},
            {"name":"u-boot.img","sha256":"96414fabe62bc3c15d98b78534e03c631b2a2f4a0d7541e435832c1dcb82c6d3"},
            {"name":"future-stage.bin","sha256":"0000000000000000000000000000000000000000000000000000000000000001"}
        ]}"#;

        let manifest = with_required(|req| parse_boot_manifest(body, req)).unwrap();
        assert_eq!(manifest.artifacts().len(), 3);
    }

    #[test]
    fn the_same_artifact_published_twice_with_different_hashes_is_refused() {
        let body = br#"{"files":[
            {"name":"tiboot3.bin","sha256":"b677db13afd0b4104fdbd17281c20fec234fbb4d6908647e3a940ea07a0c2acc"},
            {"name":"tiboot3.bin","sha256":"0000000000000000000000000000000000000000000000000000000000000002"},
            {"name":"tispl.bin","sha256":"cb0cdea0bd4c6eb430905702069b74719a5081755ea1a3e93b0dac526c16fd0e"},
            {"name":"u-boot.img","sha256":"96414fabe62bc3c15d98b78534e03c631b2a2f4a0d7541e435832c1dcb82c6d3"}
        ]}"#;

        assert!(matches!(
            with_required(|req| parse_boot_manifest(body, req)),
            Err(BootManifestError::ConflictingArtifact(_))
        ));
    }

    #[test]
    fn an_identical_duplicate_is_also_refused() {
        let body = br#"{"files":[
            {"name":"tiboot3.bin","sha256":"b677db13afd0b4104fdbd17281c20fec234fbb4d6908647e3a940ea07a0c2acc"},
            {"name":"tiboot3.bin","sha256":"b677db13afd0b4104fdbd17281c20fec234fbb4d6908647e3a940ea07a0c2acc"},
            {"name":"tispl.bin","sha256":"cb0cdea0bd4c6eb430905702069b74719a5081755ea1a3e93b0dac526c16fd0e"},
            {"name":"u-boot.img","sha256":"96414fabe62bc3c15d98b78534e03c631b2a2f4a0d7541e435832c1dcb82c6d3"}
        ]}"#;

        assert!(matches!(
            with_required(|req| parse_boot_manifest(body, req)),
            Err(BootManifestError::ConflictingArtifact(_))
        ));
    }

    #[test]
    fn a_stored_manifest_missing_a_stage_cannot_be_reconstructed() {
        let stored = vec![BootArtifact {
            name: "tiboot3.bin".to_owned(),
            sha256: Sha256::parse(
                "b677db13afd0b4104fdbd17281c20fec234fbb4d6908647e3a940ea07a0c2acc",
            )
            .unwrap(),
        }];

        assert!(matches!(
            with_required(|req| VerifiedBootManifest::from_stored(stored.clone(), req)),
            Err(BootManifestError::MissingArtifact(_))
        ));
    }
}
