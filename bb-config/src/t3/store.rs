//! Versioned, persistent SQLite storage for a validated T3 catalog (`instruction.md` §6.4).
//!
//! Three things this module deliberately does *not* do:
//!
//! * It never uses a temporary database. The caller supplies a path inside the application data
//!   directory, so a cached catalog survives a restart.
//! * It never drops and recreates tables to "migrate". Schema changes go through
//!   [`MIGRATIONS`], keyed on `PRAGMA user_version`, so stored rows are carried forward.
//! * It never collapses the four integrity gates. `archive_sha256`, `archive_size`,
//!   `extracted_sha256` and `extracted_size` are four separate columns.
//!
//! Resolving *where* the application data directory lives is the front-end's job; this module only
//! takes a path, so the library gains no platform-directory dependency.

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;
use std::time::Duration;

use chrono::NaiveDate;
use rusqlite::{Connection, OptionalExtension, params};
use url::Url;

use crate::t3::boot_manifest::{BootArtifact, VerifiedBootManifest};
use crate::t3::canonical::{
    Board, BoardCapabilities, CustomizationProfile, DfuProfile, DfuStageSpec, Image,
    ImageIntegrity, MatchingType, ProductScope, T3InitFormat,
};
use crate::t3::sha256::Sha256;
use crate::t3::validate::{CatalogProvenance, ValidatedT3Catalog};

/// Schema version this build writes and expects.
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

/// Ordered schema migrations.
///
/// Each entry is `(target_version, sql)` and is applied inside a transaction when the database's
/// `user_version` is below `target_version`. Adding a migration means appending here and bumping
/// [`CURRENT_SCHEMA_VERSION`]; existing rows must be preserved by the SQL, never recreated.
pub const MIGRATIONS: &[(u32, &str)] = &[(1, SCHEMA_V1), (2, SCHEMA_V2)];

const SCHEMA_V1: &str = r#"
CREATE TABLE catalog_provenance (
    id               INTEGER PRIMARY KEY CHECK (id = 1),
    source_url       TEXT    NOT NULL,
    latest_version   TEXT,
    fetched_at       TEXT    NOT NULL,
    product_scope    TEXT    NOT NULL
);

CREATE TABLE boards (
    id                        INTEGER PRIMARY KEY,
    name                      TEXT    NOT NULL UNIQUE,
    description               TEXT    NOT NULL,
    icon                      TEXT,
    matching_type             TEXT    NOT NULL,
    is_default                INTEGER NOT NULL,
    in_product_scope          INTEGER NOT NULL,
    sd                        INTEGER NOT NULL,
    dfu_vendor_id             INTEGER,
    dfu_product_id            INTEGER,
    dfu_boot_manifest         TEXT,
    dfu_raw_emmc_alt_setting  TEXT
);

CREATE TABLE board_tags (
    board_id INTEGER NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
    tag      TEXT    NOT NULL,
    PRIMARY KEY (board_id, tag)
);

CREATE TABLE dfu_stages (
    board_id             INTEGER NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
    position             INTEGER NOT NULL,
    artifact_name        TEXT    NOT NULL,
    alt_setting          TEXT    NOT NULL,
    reset_after          INTEGER NOT NULL,
    reconnect_timeout_ms INTEGER NOT NULL,
    PRIMARY KEY (board_id, position)
);

CREATE TABLE images (
    id               INTEGER PRIMARY KEY,
    name             TEXT    NOT NULL,
    description      TEXT    NOT NULL,
    icon             TEXT,
    -- Deliberately NOT unique. The live catalog publishes the same Ubuntu images twice: once at
    -- the top level and once inside the "Ubuntu Images" sub-list. Both entries are real listings
    -- the user can pick, so an image row is identified by (url, image_group), not by url alone.
    url              TEXT    NOT NULL,
    archive_sha256   TEXT    NOT NULL,
    archive_size     INTEGER,
    extracted_sha256 TEXT    NOT NULL,
    extracted_size   INTEGER NOT NULL,
    release_date     TEXT    NOT NULL,
    init_format      TEXT,
    desktop_variant  INTEGER,
    image_group      TEXT
);

CREATE TABLE image_devices (
    image_id INTEGER NOT NULL REFERENCES images(id) ON DELETE CASCADE,
    tag      TEXT    NOT NULL,
    PRIMARY KEY (image_id, tag)
);

CREATE INDEX idx_board_tags_tag    ON board_tags(tag);
CREATE INDEX idx_image_devices_tag ON image_devices(tag);
"#;

/// v2 (`instruction.md` §8.3): remember *how* the last-known-good documents were fetched, and keep
/// a verified boot manifest across restarts.
///
/// `ALTER TABLE ... ADD COLUMN` is used rather than a table rebuild so the catalog a user already
/// has on disk survives the upgrade — the whole point of a last-known-good cache.
const SCHEMA_V2: &str = r#"
ALTER TABLE catalog_provenance ADD COLUMN etag TEXT;
ALTER TABLE catalog_provenance ADD COLUMN last_modified TEXT;

CREATE TABLE boot_manifests (
    board_tag     TEXT PRIMARY KEY,
    source_url    TEXT NOT NULL,
    fetched_at    TEXT NOT NULL,
    etag          TEXT,
    last_modified TEXT
);

-- One row per artifact, ordered. A manifest is only ever written whole, inside the same
-- transaction as its parent row, so a partially stored manifest cannot be read back.
CREATE TABLE boot_manifest_artifacts (
    board_tag TEXT    NOT NULL REFERENCES boot_manifests(board_tag) ON DELETE CASCADE,
    position  INTEGER NOT NULL,
    name      TEXT    NOT NULL,
    sha256    TEXT    NOT NULL,
    PRIMARY KEY (board_tag, position)
);
"#;

/// The HTTP cache validators a document was last fetched with (`instruction.md` §8.2).
///
/// Storing them is what makes a refresh conditional: the client can ask "has this changed?" and
/// keep the verified copy it already has when the answer is no.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HttpValidators {
    /// `ETag` response header, verbatim.
    pub etag: Option<String>,
    /// `Last-Modified` response header, verbatim.
    pub last_modified: Option<String>,
}

impl HttpValidators {
    /// Whether a conditional request can be made at all.
    pub const fn is_empty(&self) -> bool {
        self.etag.is_none() && self.last_modified.is_none()
    }
}

/// Why a catalog store operation failed.
#[derive(Debug)]
pub enum StoreError {
    /// The underlying SQLite call failed.
    Sqlite(rusqlite::Error),
    /// The database was written by a newer build.
    ///
    /// Downgrading is not attempted; the caller should surface this and offer a controlled reset
    /// rather than risk interpreting unknown columns.
    FutureSchema {
        /// Version found on disk.
        found: u32,
        /// Version this build understands.
        supported: u32,
    },
    /// A stored row could not be turned back into a canonical value.
    Corrupt {
        /// Which table the bad row came from.
        table: &'static str,
        /// What was wrong with it.
        reason: String,
    },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(err) => write!(f, "catalog store sqlite error: {err}"),
            Self::FutureSchema { found, supported } => write!(
                f,
                "catalog store schema v{found} was written by a newer build \
                 (this build supports v{supported})"
            ),
            Self::Corrupt { table, reason } => write!(
                f,
                "catalog store table `{table}` holds an unusable row: {reason}"
            ),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(err) => Some(err),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Sqlite(err)
    }
}

/// A persistent, versioned catalog cache.
#[derive(Debug)]
pub struct T3CatalogStore {
    connection: Connection,
}

impl T3CatalogStore {
    /// Open (or create) a store at `path`, migrating it to [`CURRENT_SCHEMA_VERSION`].
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        Self::from_connection(Connection::open(path)?)
    }

    /// Open an in-memory store. Intended for tests.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self, StoreError> {
        connection.execute_batch("PRAGMA foreign_keys = ON;")?;
        let mut store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    /// Schema version currently on disk.
    pub fn schema_version(&self) -> Result<u32, StoreError> {
        Ok(self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))? as u32)
    }

    /// Apply any outstanding migrations.
    ///
    /// Each migration runs in its own transaction, so an interrupted upgrade leaves the database at
    /// the last fully applied version rather than half-migrated.
    fn migrate(&mut self) -> Result<(), StoreError> {
        let current = self.schema_version()?;
        if current > CURRENT_SCHEMA_VERSION {
            return Err(StoreError::FutureSchema {
                found: current,
                supported: CURRENT_SCHEMA_VERSION,
            });
        }

        for (version, sql) in MIGRATIONS {
            if *version <= current {
                continue;
            }
            let tx = self.connection.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", *version as i64)?;
            tx.commit()?;
        }

        Ok(())
    }

    /// Replace the stored catalog with `catalog`.
    ///
    /// The whole write runs in one transaction: a failure part-way leaves the previous
    /// last-known-good catalog intact rather than a half-written one.
    pub fn save(
        &mut self,
        catalog: &ValidatedT3Catalog,
        scope: ProductScope,
        fetched_at: NaiveDate,
    ) -> Result<(), StoreError> {
        self.save_with_validators(catalog, scope, fetched_at, &HttpValidators::default())
    }

    /// Replace the stored catalog and remember the HTTP validators it arrived with.
    ///
    /// Only a catalog that has already been through [`crate::t3::validate`] can be passed here, so
    /// "stored" and "validated" cannot drift apart: an unparsable refresh never reaches this
    /// method and therefore never displaces the last-known-good copy (`instruction.md` §8.3).
    pub fn save_with_validators(
        &mut self,
        catalog: &ValidatedT3Catalog,
        scope: ProductScope,
        fetched_at: NaiveDate,
        validators: &HttpValidators,
    ) -> Result<(), StoreError> {
        let tx = self.connection.transaction()?;

        tx.execute("DELETE FROM images", [])?;
        tx.execute("DELETE FROM boards", [])?;
        tx.execute("DELETE FROM catalog_provenance", [])?;

        tx.execute(
            "INSERT INTO catalog_provenance
                 (id, source_url, latest_version, fetched_at, product_scope, etag, last_modified)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                catalog.provenance.source_url,
                catalog.provenance.latest_version,
                fetched_at.to_string(),
                scope_to_str(scope),
                validators.etag,
                validators.last_modified,
            ],
        )?;

        for board in &catalog.boards {
            insert_board(&tx, board)?;
        }
        for image in &catalog.images {
            insert_image(&tx, image)?;
        }

        tx.commit()?;
        Ok(())
    }

    /// Load the stored catalog, or `None` when nothing has been saved yet.
    pub fn load(&self) -> Result<Option<ValidatedT3Catalog>, StoreError> {
        let Some(provenance) = self.load_provenance()? else {
            return Ok(None);
        };

        Ok(Some(ValidatedT3Catalog {
            boards: self.load_boards()?,
            images: self.load_images()?,
            provenance,
        }))
    }

    /// Date the stored catalog was fetched, for the "showing a catalog from …" UI state.
    pub fn stored_fetched_at(&self) -> Result<Option<NaiveDate>, StoreError> {
        let raw: Option<String> = self
            .connection
            .query_row(
                "SELECT fetched_at FROM catalog_provenance WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;

        raw.map(|value| {
            value
                .parse::<NaiveDate>()
                .map_err(|err| StoreError::Corrupt {
                    table: "catalog_provenance",
                    reason: format!("fetched_at `{value}` is not a date: {err}"),
                })
        })
        .transpose()
    }

    /// HTTP validators the stored catalog was fetched with, if any.
    pub fn stored_validators(&self) -> Result<Option<HttpValidators>, StoreError> {
        Ok(self
            .connection
            .query_row(
                "SELECT etag, last_modified FROM catalog_provenance WHERE id = 1",
                [],
                |row| {
                    Ok(HttpValidators {
                        etag: row.get(0)?,
                        last_modified: row.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// Store a boot manifest that has already been verified.
    ///
    /// The type system carries the guarantee: a [`VerifiedBootManifest`] cannot be built from a
    /// manifest that is missing a stage, so nothing incomplete can be written here.
    pub fn save_boot_manifest(
        &mut self,
        board_tag: &str,
        source_url: &str,
        manifest: &VerifiedBootManifest,
        fetched_at: NaiveDate,
        validators: &HttpValidators,
    ) -> Result<(), StoreError> {
        let tx = self.connection.transaction()?;

        tx.execute(
            "DELETE FROM boot_manifests WHERE board_tag = ?1",
            params![board_tag],
        )?;
        tx.execute(
            "INSERT INTO boot_manifests (board_tag, source_url, fetched_at, etag, last_modified)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                board_tag,
                source_url,
                fetched_at.to_string(),
                validators.etag,
                validators.last_modified,
            ],
        )?;

        for (position, artifact) in manifest.artifacts().iter().enumerate() {
            tx.execute(
                "INSERT INTO boot_manifest_artifacts (board_tag, position, name, sha256)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    board_tag,
                    position as i64,
                    artifact.name,
                    artifact.sha256.to_string(),
                ],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// Load the last-known-good boot manifest for a board.
    ///
    /// Returns `Ok(None)` when nothing has been stored, and an error when what is stored no longer
    /// satisfies `required`. Neither result is a manifest, and `instruction.md` §8.3 is explicit
    /// that without a verified manifest DFU does not start — there is deliberately no way to get a
    /// partial one out of this method.
    pub fn load_boot_manifest(
        &self,
        board_tag: &str,
        required: &[&str],
    ) -> Result<Option<VerifiedBootManifest>, StoreError> {
        let exists: Option<i64> = self
            .connection
            .query_row(
                "SELECT 1 FROM boot_manifests WHERE board_tag = ?1",
                params![board_tag],
                |row| row.get(0),
            )
            .optional()?;

        if exists.is_none() {
            return Ok(None);
        }

        let mut stmt = self.connection.prepare(
            "SELECT name, sha256 FROM boot_manifest_artifacts
             WHERE board_tag = ?1 ORDER BY position",
        )?;
        let stored = stmt
            .query_map(params![board_tag], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let artifacts = stored
            .into_iter()
            .map(|(name, sha256)| {
                Sha256::parse(&sha256)
                    .map(|sha256| BootArtifact { name, sha256 })
                    .map_err(|err| StoreError::Corrupt {
                        table: "boot_manifest_artifacts",
                        reason: format!("sha256 `{sha256}` is unusable: {err}"),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;

        VerifiedBootManifest::from_stored(artifacts, required)
            .map(Some)
            .map_err(|err| StoreError::Corrupt {
                table: "boot_manifest_artifacts",
                reason: err.to_string(),
            })
    }

    /// Product scope the stored catalog was validated against.
    pub fn stored_scope(&self) -> Result<Option<ProductScope>, StoreError> {
        self.connection
            .query_row(
                "SELECT product_scope FROM catalog_provenance WHERE id = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|raw| scope_from_str(&raw))
            .transpose()
    }

    fn load_provenance(&self) -> Result<Option<CatalogProvenance>, StoreError> {
        Ok(self
            .connection
            .query_row(
                "SELECT source_url, latest_version FROM catalog_provenance WHERE id = 1",
                [],
                |row| {
                    Ok(CatalogProvenance {
                        source_url: row.get(0)?,
                        latest_version: row.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    fn load_boards(&self) -> Result<Vec<Board>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, description, icon, matching_type, is_default, in_product_scope,
                    sd, dfu_vendor_id, dfu_product_id, dfu_boot_manifest, dfu_raw_emmc_alt_setting
             FROM boards ORDER BY id",
        )?;

        let rows: Vec<StoredBoard> = statement
            .query_map([], StoredBoard::from_row)?
            .collect::<rusqlite::Result<_>>()?;

        rows.into_iter()
            .map(|stored| self.hydrate_board(stored))
            .collect()
    }

    fn hydrate_board(&self, stored: StoredBoard) -> Result<Board, StoreError> {
        let tags = self.load_tags("board_tags", "board_id", stored.id)?;

        let emmc_dfu = match (
            stored.dfu_vendor_id,
            stored.dfu_product_id,
            stored.dfu_boot_manifest.as_deref(),
            stored.dfu_raw_emmc_alt_setting.clone(),
        ) {
            (Some(vendor_id), Some(product_id), Some(manifest), Some(raw_emmc_alt_setting)) => {
                let boot_manifest = Url::parse(manifest).map_err(|err| StoreError::Corrupt {
                    table: "boards",
                    reason: format!("dfu_boot_manifest is not a URL: {err}"),
                })?;
                Some(DfuProfile {
                    vendor_id,
                    product_id,
                    boot_manifest,
                    stages: self.load_dfu_stages(stored.id)?,
                    raw_emmc_alt_setting,
                })
            }
            _ => None,
        };

        Ok(Board {
            name: stored.name,
            description: stored.description,
            tags,
            icon: parse_optional_url(stored.icon.as_deref(), "boards")?,
            capabilities: BoardCapabilities {
                sd: stored.sd,
                emmc_dfu,
            },
            matching_type: MatchingType::parse(&stored.matching_type),
            is_default: stored.is_default,
            in_product_scope: stored.in_product_scope,
        })
    }

    fn load_dfu_stages(&self, board_id: i64) -> Result<Vec<DfuStageSpec>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT artifact_name, alt_setting, reset_after, reconnect_timeout_ms
             FROM dfu_stages WHERE board_id = ?1 ORDER BY position",
        )?;

        let stages = statement
            .query_map([board_id], |row| {
                Ok(DfuStageSpec {
                    artifact_name: row.get(0)?,
                    alt_setting: row.get(1)?,
                    reset_after: row.get(2)?,
                    reconnect_timeout: Duration::from_millis(row.get::<_, i64>(3)? as u64),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;

        Ok(stages)
    }

    fn load_images(&self) -> Result<Vec<Image>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, description, icon, url, archive_sha256, archive_size,
                    extracted_sha256, extracted_size, release_date, init_format,
                    desktop_variant, image_group
             FROM images ORDER BY id",
        )?;

        let rows: Vec<StoredImage> = statement
            .query_map([], StoredImage::from_row)?
            .collect::<rusqlite::Result<_>>()?;

        rows.into_iter()
            .map(|stored| self.hydrate_image(stored))
            .collect()
    }

    fn hydrate_image(&self, stored: StoredImage) -> Result<Image, StoreError> {
        let devices = self.load_tags("image_devices", "image_id", stored.id)?;

        let customization = match stored.init_format.as_deref() {
            Some(raw) => match T3InitFormat::parse(raw) {
                Some(init_format) => Some(CustomizationProfile {
                    init_format,
                    desktop_variant: stored.desktop_variant.unwrap_or(false),
                }),
                None => {
                    return Err(StoreError::Corrupt {
                        table: "images",
                        reason: format!("unknown init_format `{raw}`"),
                    });
                }
            },
            None => None,
        };

        let release_date =
            stored
                .release_date
                .parse::<NaiveDate>()
                .map_err(|_| StoreError::Corrupt {
                    table: "images",
                    reason: format!("release_date `{}` is not a date", stored.release_date),
                })?;

        Ok(Image {
            name: stored.name,
            description: stored.description,
            icon: parse_optional_url(stored.icon.as_deref(), "images")?,
            url: Url::parse(&stored.url).map_err(|err| StoreError::Corrupt {
                table: "images",
                reason: format!("url is not a URL: {err}"),
            })?,
            integrity: ImageIntegrity {
                archive_sha256: parse_stored_hash(&stored.archive_sha256, "archive_sha256")?,
                archive_size: stored
                    .archive_size
                    .map(|value| size_from_sql(value, "archive_size"))
                    .transpose()?,
                extracted_sha256: parse_stored_hash(&stored.extracted_sha256, "extracted_sha256")?,
                extracted_size: size_from_sql(stored.extracted_size, "extracted_size")?,
            },
            release_date,
            devices,
            customization,
            group: stored.image_group,
        })
    }

    fn load_tags(
        &self,
        table: &str,
        key_column: &str,
        id: i64,
    ) -> Result<BTreeSet<String>, StoreError> {
        // `table` and `key_column` are compile-time literals from this module, never user input.
        let sql = format!("SELECT tag FROM {table} WHERE {key_column} = ?1 ORDER BY tag");
        let mut statement = self.connection.prepare(&sql)?;
        let tags = statement
            .query_map([id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(tags)
    }
}

fn insert_board(tx: &rusqlite::Transaction<'_>, board: &Board) -> Result<(), StoreError> {
    let dfu = board.capabilities.emmc_dfu.as_ref();

    tx.execute(
        "INSERT INTO boards (name, description, icon, matching_type, is_default, in_product_scope,
                             sd, dfu_vendor_id, dfu_product_id, dfu_boot_manifest,
                             dfu_raw_emmc_alt_setting)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            board.name,
            board.description,
            board.icon.as_ref().map(Url::to_string),
            matching_type_to_str(board.matching_type),
            board.is_default,
            board.in_product_scope,
            board.capabilities.sd,
            dfu.map(|profile| profile.vendor_id),
            dfu.map(|profile| profile.product_id),
            dfu.map(|profile| profile.boot_manifest.to_string()),
            dfu.map(|profile| profile.raw_emmc_alt_setting.clone()),
        ],
    )?;
    let board_id = tx.last_insert_rowid();

    for tag in &board.tags {
        tx.execute(
            "INSERT INTO board_tags (board_id, tag) VALUES (?1, ?2)",
            params![board_id, tag],
        )?;
    }

    if let Some(profile) = dfu {
        for (position, stage) in profile.stages.iter().enumerate() {
            tx.execute(
                "INSERT INTO dfu_stages (board_id, position, artifact_name, alt_setting,
                                         reset_after, reconnect_timeout_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    board_id,
                    position as i64,
                    stage.artifact_name,
                    stage.alt_setting,
                    stage.reset_after,
                    stage.reconnect_timeout.as_millis() as i64,
                ],
            )?;
        }
    }

    Ok(())
}

fn insert_image(tx: &rusqlite::Transaction<'_>, image: &Image) -> Result<(), StoreError> {
    // SQLite integers are signed 64-bit, so sizes are converted explicitly rather than with `as`,
    // which would silently wrap a value past `i64::MAX` into a negative row.
    let archive_size = image
        .integrity
        .archive_size
        .map(|value| size_to_sql(value, "archive_size"))
        .transpose()?;
    let extracted_size = size_to_sql(image.integrity.extracted_size, "extracted_size")?;

    tx.execute(
        "INSERT INTO images (name, description, icon, url, archive_sha256, archive_size,
                             extracted_sha256, extracted_size, release_date, init_format,
                             desktop_variant, image_group)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            image.name,
            image.description,
            image.icon.as_ref().map(Url::to_string),
            image.url.to_string(),
            image.integrity.archive_sha256.to_hex(),
            archive_size,
            image.integrity.extracted_sha256.to_hex(),
            extracted_size,
            image.release_date.to_string(),
            image
                .customization
                .as_ref()
                .map(|profile| init_format_to_str(profile.init_format)),
            image
                .customization
                .as_ref()
                .map(|profile| profile.desktop_variant),
            image.group,
        ],
    )?;
    let image_id = tx.last_insert_rowid();

    for tag in &image.devices {
        tx.execute(
            "INSERT INTO image_devices (image_id, tag) VALUES (?1, ?2)",
            params![image_id, tag],
        )?;
    }

    Ok(())
}

/// Convert a model size into SQLite's signed integer domain, refusing to wrap.
fn size_to_sql(value: u64, field: &'static str) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::Corrupt {
        table: "images",
        reason: format!("{field} exceeds SQLite's signed 64-bit integer range"),
    })
}

/// Convert a stored size back, refusing to read a negative row as a huge unsigned value.
fn size_from_sql(value: i64, field: &'static str) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| StoreError::Corrupt {
        table: "images",
        reason: format!("{field} is negative"),
    })
}

fn parse_stored_hash(raw: &str, field: &'static str) -> Result<Sha256, StoreError> {
    Sha256::parse(raw).map_err(|err| StoreError::Corrupt {
        table: "images",
        reason: format!("{field}: {err}"),
    })
}

fn parse_optional_url(raw: Option<&str>, table: &'static str) -> Result<Option<Url>, StoreError> {
    match raw {
        None => Ok(None),
        Some(value) => Url::parse(value)
            .map(Some)
            .map_err(|err| StoreError::Corrupt {
                table,
                reason: format!("icon is not a URL: {err}"),
            }),
    }
}

fn matching_type_to_str(value: MatchingType) -> &'static str {
    match value {
        MatchingType::Exclusive => "exclusive",
        MatchingType::Inclusive => "inclusive",
    }
}

fn init_format_to_str(value: T3InitFormat) -> &'static str {
    match value {
        T3InitFormat::Systemd => "systemd",
    }
}

fn scope_to_str(value: ProductScope) -> &'static str {
    match value {
        ProductScope::T3Only => "t3-only",
        ProductScope::T3AndBeagleY => "t3-and-beagley",
    }
}

fn scope_from_str(raw: &str) -> Result<ProductScope, StoreError> {
    match raw {
        "t3-only" => Ok(ProductScope::T3Only),
        "t3-and-beagley" => Ok(ProductScope::T3AndBeagleY),
        other => Err(StoreError::Corrupt {
            table: "catalog_provenance",
            reason: format!("unknown product_scope `{other}`"),
        }),
    }
}

struct StoredBoard {
    id: i64,
    name: String,
    description: String,
    icon: Option<String>,
    matching_type: String,
    is_default: bool,
    in_product_scope: bool,
    sd: bool,
    dfu_vendor_id: Option<u16>,
    dfu_product_id: Option<u16>,
    dfu_boot_manifest: Option<String>,
    dfu_raw_emmc_alt_setting: Option<String>,
}

impl StoredBoard {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            icon: row.get(3)?,
            matching_type: row.get(4)?,
            is_default: row.get(5)?,
            in_product_scope: row.get(6)?,
            sd: row.get(7)?,
            dfu_vendor_id: row.get(8)?,
            dfu_product_id: row.get(9)?,
            dfu_boot_manifest: row.get(10)?,
            dfu_raw_emmc_alt_setting: row.get(11)?,
        })
    }
}

struct StoredImage {
    id: i64,
    name: String,
    description: String,
    icon: Option<String>,
    url: String,
    archive_sha256: String,
    archive_size: Option<i64>,
    extracted_sha256: String,
    extracted_size: i64,
    release_date: String,
    init_format: Option<String>,
    desktop_variant: Option<bool>,
    image_group: Option<String>,
}

impl StoredImage {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            icon: row.get(3)?,
            url: row.get(4)?,
            archive_sha256: row.get(5)?,
            archive_size: row.get(6)?,
            extracted_sha256: row.get(7)?,
            extracted_size: row.get(8)?,
            release_date: row.get(9)?,
            init_format: row.get(10)?,
            desktop_variant: row.get(11)?,
            image_group: row.get(12)?,
        })
    }
}
