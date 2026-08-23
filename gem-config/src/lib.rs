//! # Introduction
//!
//! T3 Gemstone publishes a json catalog listing every board image, which the imager reads to get
//! the latest images for each board. See [`t3`] for that schema and its strict validator.
//!
//! [`config`] additionally parses the upstream BeagleBoard.org `distros.json` schema this project
//! was derived from; [`t3::bridge`] adapts the T3 catalog onto it.

pub mod config;
pub mod t3;

pub use config::Config;

#[cfg(test)]
mod tests {
    #[test]
    fn basic() {
        let data = include_bytes!("../../config.json");
        serde_json::from_slice::<super::Config>(data).unwrap();
    }
}
