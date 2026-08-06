use std::{
    fs::OpenOptions,
    io::{self, Write as _},
    path::{Path, PathBuf},
};

use gem_config::t3::{DfuProfile, VerifiedBootManifest, parse_boot_manifest};
use gem_downloader::{ArchiveIntegrity, Downloader};
use gem_helper::{cancel::CancellationToken, file_stream::file_stream};
use serde_json::{Value, json};

use crate::{
    Error, Result, check_cancel,
    model::{DfuStage, DfuStageInput, DfuStageKind},
};

/// One boot artifact that has passed the manifest, byte-count and SHA-256 gates and is now
/// reachable under the downloader's content-addressed cache name.
#[derive(Debug, Clone)]
pub struct ResolvedBootArtifact {
    pub stage: DfuStage,
    pub path: PathBuf,
}

impl ResolvedBootArtifact {
    pub fn into_input(self) -> Result<DfuStageInput> {
        DfuStageInput::from_path(self.stage, self.path).map_err(Error::ResolverIo)
    }
}

#[derive(Debug, Clone)]
pub struct BootArtifactResolver {
    downloader: Downloader,
    manifest_cache: PathBuf,
}

impl BootArtifactResolver {
    pub fn new(downloader: Downloader, manifest_cache: impl Into<PathBuf>) -> Self {
        Self {
            downloader,
            manifest_cache: manifest_cache.into(),
        }
    }

    pub async fn resolve(
        &self,
        profile: &DfuProfile,
        cancel: Option<&CancellationToken>,
    ) -> Result<Vec<ResolvedBootArtifact>> {
        check_cancel(cancel)?;
        let required = profile.required_artifacts();
        let manifest = match self.fetch_manifest(profile, &required).await {
            Ok((manifest, raw)) => {
                publish_manifest(&self.manifest_cache, profile.boot_manifest.as_str(), &raw)?;
                manifest
            }
            Err(remote_error) => {
                self.load_last_known_good(profile, &required)
                    .map_err(|cache_error| Error::NoVerifiedBootManifest {
                        remote: Box::new(remote_error),
                        cache: Box::new(cache_error),
                    })?
            }
        };

        let mut resolved = Vec::with_capacity(profile.stages.len());
        for (index, spec) in profile.stages.iter().enumerate() {
            check_cancel(cancel)?;
            let artifact = manifest.artifact(&spec.artifact_name).ok_or_else(|| {
                Error::InvalidPlan(format!(
                    "verified manifest lost required artifact `{}`",
                    spec.artifact_name
                ))
            })?;
            let url = profile
                .boot_manifest
                .join(&artifact.name)
                .map_err(Error::BootArtifactUrl)?;
            let integrity = ArchiveIntegrity::from_sha256(*artifact.sha256.as_bytes());

            let path = if let Some(path) = self
                .downloader
                .check_cache_from_sha(*artifact.sha256.as_bytes())
            {
                path
            } else {
                let (writer, reader) = file_stream().map_err(Error::ResolverIo)?;
                self.downloader
                    .download_to_stream(url, integrity, writer)
                    .await
                    .map_err(Error::BootArtifactDownload)?;
                drop(reader);
                self.downloader
                    .check_cache_from_sha(*artifact.sha256.as_bytes())
                    .ok_or_else(|| {
                        Error::InvalidPlan(format!(
                            "verified artifact `{}` was not published to cache",
                            artifact.name
                        ))
                    })?
            };
            check_cancel(cancel)?;
            let size = std::fs::metadata(&path).map_err(Error::ResolverIo)?.len();
            if size == 0 {
                return Err(Error::EmptyImage(artifact.name.clone()));
            }
            let next_alt_setting = profile
                .stages
                .get(index + 1)
                .map(|next| next.alt_setting.clone())
                .unwrap_or_else(|| profile.raw_emmc_alt_setting.clone());
            resolved.push(ResolvedBootArtifact {
                stage: DfuStage {
                    kind: DfuStageKind::BootArtifact { next_alt_setting },
                    artifact_name: spec.artifact_name.clone(),
                    alt_setting: spec.alt_setting.clone(),
                    reset_after: spec.reset_after,
                    reconnect_timeout: spec.reconnect_timeout,
                    expected_sha256: *artifact.sha256.as_bytes(),
                    expected_size: Some(size),
                },
                path,
            });
        }
        Ok(resolved)
    }

    pub fn resolve_blocking(
        &self,
        profile: &DfuProfile,
        cancel: Option<&CancellationToken>,
    ) -> Result<Vec<ResolvedBootArtifact>> {
        // `enable_all` is not optional here. Without the IO and time drivers the first request
        // this runtime makes panics with "there is no reactor running" instead of returning an
        // error — and a panic on a blocking worker never reaches the failure screen, so the write
        // stops dead on whatever phase was last announced.
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(Error::ResolverIo)?
            .block_on(self.resolve(profile, cancel))
    }

    async fn fetch_manifest(
        &self,
        profile: &DfuProfile,
        required: &[&str],
    ) -> Result<(VerifiedBootManifest, Vec<u8>)> {
        let value: Value = self
            .downloader
            .download_json_no_cache(profile.boot_manifest.clone())
            .await
            .map_err(Error::BootManifestDownload)?;
        let raw = serde_json::to_vec(&value).map_err(Error::BootManifestSerialize)?;
        let manifest = parse_boot_manifest(&raw, required).map_err(Error::BootManifest)?;
        Ok((manifest, raw))
    }

    fn load_last_known_good(
        &self,
        profile: &DfuProfile,
        required: &[&str],
    ) -> Result<VerifiedBootManifest> {
        let bytes = std::fs::read(&self.manifest_cache).map_err(Error::ResolverIo)?;
        let envelope: Value =
            serde_json::from_slice(&bytes).map_err(Error::BootManifestSerialize)?;
        let source = envelope
            .get("source_url")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::InvalidPlan("cached boot manifest has no source_url".to_owned())
            })?;
        if source != profile.boot_manifest.as_str() {
            return Err(Error::InvalidPlan(format!(
                "cached boot manifest source `{source}` does not match `{}`",
                profile.boot_manifest
            )));
        }
        let body = envelope.get("manifest").ok_or_else(|| {
            Error::InvalidPlan("cached boot manifest has no manifest body".to_owned())
        })?;
        let raw = serde_json::to_vec(body).map_err(Error::BootManifestSerialize)?;
        parse_boot_manifest(&raw, required).map_err(Error::BootManifest)
    }
}

fn publish_manifest(path: &Path, source_url: &str, raw: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        Error::ResolverIo(io::Error::new(
            io::ErrorKind::InvalidInput,
            "manifest cache path has no parent",
        ))
    })?;
    std::fs::create_dir_all(parent).map_err(Error::ResolverIo)?;
    let manifest: Value = serde_json::from_slice(raw).map_err(Error::BootManifestSerialize)?;
    let body = serde_json::to_vec(&json!({
        "source_url": source_url,
        "manifest": manifest,
    }))
    .map_err(Error::BootManifestSerialize)?;
    let scratch = path.with_extension(format!("part-{}", std::process::id()));
    let result = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&scratch)?;
        file.write_all(&body)?;
        file.flush()?;
        file.sync_all()?;
        std::fs::rename(&scratch, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&scratch);
    }
    result.map_err(Error::ResolverIo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gem_config::t3::DfuProfile;
    use gem_downloader::TransportPolicy;
    use httpmock::prelude::*;
    use sha2::{Digest as _, Sha256};

    fn hex(data: &[u8]) -> String {
        const_hex::encode(Sha256::digest(data))
    }

    #[tokio::test]
    async fn resolves_exactly_three_verified_artifacts_in_contract_order() {
        let server = MockServer::start();
        let payloads = [
            ("tiboot3.bin", b"boot".as_slice()),
            ("tispl.bin", b"spl".as_slice()),
            ("u-boot.img", b"uboot".as_slice()),
        ];
        let manifest = json!({"files": payloads.iter().map(|(name, bytes)| {
            json!({"name": name, "sha256": hex(bytes)})
        }).collect::<Vec<_>>()});
        server.mock(|when, then| {
            when.method(GET).path("/list.json");
            then.status(200).json_body(manifest.clone());
        });
        for (name, bytes) in payloads {
            server.mock(|when, then| {
                when.method(GET).path(format!("/{name}"));
                then.status(200).body(bytes);
            });
        }

        let temp = tempfile::tempdir().unwrap();
        let downloader = Downloader::with_policy(
            temp.path().join("objects"),
            TransportPolicy::plaintext_for_tests(),
        )
        .unwrap();
        let resolver = BootArtifactResolver::new(downloader, temp.path().join("manifest.json"));
        let mut profile = DfuProfile::t3_gem_o1();
        profile.boot_manifest = format!("{}/list.json", server.base_url()).parse().unwrap();
        let resolved = resolver.resolve(&profile, None).await.unwrap();
        assert_eq!(resolved.len(), 3);
        assert_eq!(resolved[0].stage.artifact_name, "tiboot3.bin");
        assert_eq!(resolved[0].stage.alt_setting, "bootloader");
        assert_eq!(resolved[2].stage.artifact_name, "u-boot.img");
        assert_eq!(
            resolved[2].stage.kind,
            DfuStageKind::BootArtifact {
                next_alt_setting: "rawemmc".to_owned()
            }
        );
    }

    #[tokio::test]
    async fn manifest_or_artifact_failure_is_fail_closed_without_lkg() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/list.json");
            then.status(500);
        });
        let temp = tempfile::tempdir().unwrap();
        let downloader = Downloader::with_policy(
            temp.path().join("objects"),
            TransportPolicy::plaintext_for_tests(),
        )
        .unwrap();
        let resolver = BootArtifactResolver::new(downloader, temp.path().join("missing.json"));
        let mut profile = DfuProfile::t3_gem_o1();
        profile.boot_manifest = format!("{}/list.json", server.base_url()).parse().unwrap();
        assert!(matches!(
            resolver.resolve(&profile, None).await,
            Err(Error::NoVerifiedBootManifest { .. })
        ));
    }

    #[tokio::test]
    async fn verified_last_known_good_manifest_and_objects_survive_remote_outage() {
        let server = MockServer::start();
        let payloads = [
            ("tiboot3.bin", b"boot".as_slice()),
            ("tispl.bin", b"spl".as_slice()),
            ("u-boot.img", b"uboot".as_slice()),
        ];
        let manifest = json!({"files": payloads.iter().map(|(name, bytes)| {
            json!({"name": name, "sha256": hex(bytes)})
        }).collect::<Vec<_>>()});
        let mut manifest_mock = server.mock(|when, then| {
            when.method(GET).path("/list.json");
            then.status(200).json_body(manifest.clone());
        });
        for (name, bytes) in payloads {
            server.mock(|when, then| {
                when.method(GET).path(format!("/{name}"));
                then.status(200).body(bytes);
            });
        }
        let temp = tempfile::tempdir().unwrap();
        let downloader = Downloader::with_policy(
            temp.path().join("objects"),
            TransportPolicy::plaintext_for_tests(),
        )
        .unwrap();
        let resolver =
            BootArtifactResolver::new(downloader.clone(), temp.path().join("manifest.json"));
        let mut profile = DfuProfile::t3_gem_o1();
        profile.boot_manifest = format!("{}/list.json", server.base_url()).parse().unwrap();
        resolver.resolve(&profile, None).await.unwrap();

        manifest_mock.delete();
        server.mock(|when, then| {
            when.method(GET).path("/list.json");
            then.status(503);
        });
        let fallback = resolver.resolve(&profile, None).await.unwrap();
        assert_eq!(fallback.len(), 3);
        assert!(fallback.iter().all(|artifact| artifact.path.is_file()));
    }

    #[tokio::test]
    async fn artifact_hash_mismatch_stops_before_usb() {
        let server = MockServer::start();
        let payloads = [
            ("tiboot3.bin", b"boot".as_slice()),
            ("tispl.bin", b"spl".as_slice()),
            ("u-boot.img", b"uboot".as_slice()),
        ];
        let manifest = json!({"files": payloads.iter().map(|(name, bytes)| {
            let digest = if *name == "tispl.bin" { hex(b"different") } else { hex(bytes) };
            json!({"name": name, "sha256": digest})
        }).collect::<Vec<_>>()});
        server.mock(|when, then| {
            when.method(GET).path("/list.json");
            then.status(200).json_body(manifest.clone());
        });
        for (name, bytes) in payloads {
            server.mock(|when, then| {
                when.method(GET).path(format!("/{name}"));
                then.status(200).body(bytes);
            });
        }
        let temp = tempfile::tempdir().unwrap();
        let downloader = Downloader::with_policy(
            temp.path().join("objects"),
            TransportPolicy::plaintext_for_tests(),
        )
        .unwrap();
        let resolver = BootArtifactResolver::new(downloader, temp.path().join("manifest.json"));
        let mut profile = DfuProfile::t3_gem_o1();
        profile.boot_manifest = format!("{}/list.json", server.base_url()).parse().unwrap();
        assert!(matches!(
            resolver.resolve(&profile, None).await,
            Err(Error::BootArtifactDownload(
                gem_downloader::DownloadError::ArchiveHashMismatch { .. }
            ))
        ));
    }
}
