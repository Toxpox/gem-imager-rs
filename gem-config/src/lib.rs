pub mod config;
pub mod t3;

pub use config::Config;

#[cfg(test)]
mod tests {
    fn shipped_config() -> super::Config {
        let data = include_bytes!("../../config.json");
        serde_json::from_slice::<super::Config>(data).expect("the shipped config must parse")
    }

    #[test]
    fn basic() {
        shipped_config();
    }

    #[test]
    fn every_shipped_board_survives_parsing() {
        let raw: serde_json::Value =
            serde_json::from_slice(include_bytes!("../../config.json")).unwrap();
        let declared = raw["imager"]["devices"].as_array().map_or(0, Vec::len);

        assert_eq!(
            shipped_config().imager.devices.len(),
            declared,
            "a board was dropped while parsing; boards without a flasher field are skipped \
             silently and their images become unreachable"
        );
    }

    #[test]
    fn the_beagley_board_is_shipped_with_the_tag_its_images_use() {
        let config = shipped_config();
        let board = config
            .imager
            .devices
            .iter()
            .find(|device| device.name == "BeagleY-AI")
            .expect("BeagleY-AI must ship as a board or its images cannot be selected");

        assert!(
            board.tags.contains("beagle-am67"),
            "the official BeagleBoard catalogue tags BeagleY-AI images beagle-am67; \
             a different tag leaves the board with no images: {:?}",
            board.tags
        );
    }

    #[test]
    fn the_official_beagleboard_catalogue_is_a_configured_source() {
        let config = shipped_config();
        assert!(
            config.imager.remote_configs.iter().any(|url| {
                url.as_str()
                    == "https://raw.githubusercontent.com/beagleboard/distros/refs/heads/main/os_list.json"
            }),
            "BeagleY-AI is served from the official catalogue; without it the board has no images"
        );
    }
}
