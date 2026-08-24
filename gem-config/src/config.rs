use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_with::{Map, VecSkipError, serde_as};
use url::Url;

#[serde_as]
#[derive(Deserialize, Serialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    #[serde(default)]
    pub imager: Imager,
    #[serde_as(as = "VecSkipError<_>")]
    pub os_list: Vec<OsListItem>,
}

impl Config {
    pub fn image_count(&self) -> usize {
        fn count(items: &[OsListItem]) -> usize {
            items
                .iter()
                .map(|item| match item {
                    OsListItem::Image(_) => 1,
                    OsListItem::SubList(list) => count(&list.subitems),
                    OsListItem::RemoteSubList(_) => 0,
                })
                .sum()
        }

        count(&self.os_list)
    }
}

#[serde_as]
#[derive(Deserialize, Serialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct Imager {
    #[serde(default)]
    pub remote_configs: Vec<Url>,
    #[serde_as(as = "VecSkipError<_>")]
    #[serde(default)]
    pub devices: Vec<Device>,
}

#[serde_as]
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    pub tags: HashSet<String>,
    pub icon: Option<Url>,
    pub description: String,
    pub flasher: Flasher,
    #[serde(default)]
    pub emmc_dfu: bool,
    pub documentation: Option<Url>,
    pub instructions: Option<String>,
    #[serde(default)]
    #[serde_as(as = "Map<_, _>")]
    pub specification: Vec<(String, String)>,
    pub oshw: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
#[serde(rename_all = "lowercase")]
pub enum InitFormat {
    #[default]
    None,
    Sysconf,
    Armbian,
    CloudInit,
    GemInit,
    GemInitDesktop,
}

impl InitFormat {
    pub const fn is_gem_init(self) -> bool {
        matches!(self, Self::GemInit | Self::GemInitDesktop)
    }

    pub const fn supports_vnc(self) -> bool {
        matches!(self, Self::GemInitDesktop)
    }
}

#[cfg(feature = "store")]
impl rusqlite::ToSql for InitFormat {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        let val: u8 = match self {
            InitFormat::None => 1,
            InitFormat::Sysconf => 2,
            InitFormat::Armbian => 3,
            InitFormat::CloudInit => 4,
            InitFormat::GemInit => 5,
            InitFormat::GemInitDesktop => 6,
        };
        Ok(rusqlite::types::ToSqlOutput::from(val))
    }
}

#[cfg(feature = "store")]
impl rusqlite::types::FromSql for InitFormat {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        value.as_i64().and_then(|val| match val {
            1 => Ok(InitFormat::None),
            2 => Ok(InitFormat::Sysconf),
            3 => Ok(InitFormat::Armbian),
            4 => Ok(InitFormat::CloudInit),
            5 => Ok(InitFormat::GemInit),
            6 => Ok(InitFormat::GemInitDesktop),
            _ => Err(rusqlite::types::FromSqlError::Other(
                format!("Invalid InitFormat integer variant: {}", val).into(),
            )),
        })
    }
}

impl std::fmt::Display for InitFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InitFormat::None => f.write_str("none"),
            InitFormat::Sysconf => f.write_str("sysconfig"),
            InitFormat::Armbian => f.write_str("armbian"),
            InitFormat::CloudInit => f.write_str("cloudinit"),
            InitFormat::GemInit => f.write_str("geminit"),
            InitFormat::GemInitDesktop => f.write_str("geminit-desktop"),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
pub enum OsListItem {
    Image(OsImage),
    SubList(OsSubList),
    RemoteSubList(OsRemoteSubList),
}

#[serde_as]
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct OsSubList {
    pub name: String,
    pub description: String,
    pub icon: Url,
    #[serde(default)]
    pub flasher: Flasher,
    #[serde_as(as = "VecSkipError<_>")]
    pub subitems: Vec<OsListItem>,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct OsRemoteSubList {
    pub name: String,
    pub description: String,
    pub icon: Url,
    #[serde(default)]
    pub flasher: Flasher,
    pub devices: HashSet<String>,
    pub subitems_url: Url,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct OsImage {
    pub name: String,
    pub description: String,
    pub icon: Url,
    pub url: Url,
    pub image_download_size: Option<u64>,
    #[serde(with = "const_hex")]
    pub image_download_sha256: [u8; 32],
    pub extract_size: u64,
    #[serde(default, with = "hex_option")]
    pub extract_sha256: Option<[u8; 32]>,
    pub release_date: chrono::NaiveDate,
    pub devices: HashSet<String>,
    #[serde(default)]
    pub tags: HashSet<String>,
    #[serde(default)]
    pub init_format: InitFormat,
    pub info_text: Option<String>,
    pub support: Option<Url>,
}

mod hex_option {
    use serde::{Deserialize as _, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(
        value: &Option<[u8; 32]>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(bytes) => serializer.serialize_str(&const_hex::encode(bytes)),
            None => serializer.serialize_none(),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<[u8; 32]>, D::Error> {
        let Some(raw) = Option::<String>::deserialize(deserializer)? else {
            return Ok(None);
        };

        let mut out = [0u8; 32];
        const_hex::decode_to_slice(raw.as_str(), &mut out).map_err(serde::de::Error::custom)?;
        Ok(Some(out))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum Flasher {
    #[default]
    SdCard,
}

#[cfg(feature = "store")]
impl rusqlite::ToSql for Flasher {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        let val: u8 = match self {
            Flasher::SdCard => 1,
        };

        Ok(rusqlite::types::ToSqlOutput::from(val))
    }
}

#[cfg(feature = "store")]
impl rusqlite::types::FromSql for Flasher {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        value.as_i64().and_then(|val| match val {
            1 => Ok(Flasher::SdCard),
            _ => Err(rusqlite::types::FromSqlError::Other(
                format!("Invalid Flasher discriminant: {}", val).into(),
            )),
        })
    }
}
