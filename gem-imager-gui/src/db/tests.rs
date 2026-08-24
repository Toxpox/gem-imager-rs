use std::collections::HashSet;

use crate::constants::DEFAULT_CONFIG;

use super::*;
use gem_config::Config;

#[test]
fn init_loads_all_default_remote_configs() {
    let db = Db::new().expect("Failed to create DB");

    db.init().expect("DB initialization should succeed");

    let urls = db
        .remote_configs()
        .expect("Fetching remote configs should succeed");

    let config: Config =
        serde_json::from_slice(DEFAULT_CONFIG).expect("DEFAULT_CONFIG should be valid");

    let expected_urls = config.imager.remote_configs.clone();

    assert_eq!(
        urls.len(),
        expected_urls.len(),
        "All remote config URLs from DEFAULT_CONFIG should be inserted"
    );

    assert_eq!(expected_urls.len(), urls.len());
    for (_, u) in urls {
        assert!(expected_urls.contains(&u));
    }
}

#[test]
fn add_config_inserts_new_remote_configs() {
    let db = Db::new().expect("Failed to create DB");

    db.init().expect("DB initialization should succeed");

    let initial_urls = db
        .remote_configs()
        .expect("Fetching remote configs should succeed");

    let initial_count = initial_urls.len();

    let new_config = Config {
        imager: gem_config::config::Imager {
            remote_configs: vec![
                "https://example.com/test-os-list.json".try_into().unwrap(),
                "https://example.com/another-os-list.json"
                    .try_into()
                    .unwrap(),
            ],
            devices: vec![],
        },
        os_list: vec![],
    };

    db.add_config(new_config, None)
        .expect("add_config should succeed");

    let updated_urls = db
        .remote_configs()
        .expect("Fetching remote configs should succeed");

    assert_eq!(
        updated_urls.len(),
        initial_count + 2,
        "Two new remote configs should be added"
    );

    assert!(
        updated_urls
            .iter()
            .any(|(_, u)| u.as_str() == "https://example.com/test-os-list.json")
    );

    assert!(
        updated_urls
            .iter()
            .any(|(_, u)| u.as_str() == "https://example.com/another-os-list.json")
    );
}

#[test]
fn add_config_does_not_duplicate_remote_configs() {
    let db = Db::new().expect("Failed to create DB");

    db.init().expect("DB initialization should succeed");

    let initial_urls = db
        .remote_configs()
        .expect("Fetching remote configs should succeed");

    assert!(!initial_urls.is_empty());

    let (_, existing_url) = initial_urls.first().unwrap().clone();

    let initial_count = initial_urls.len();

    let mut imager = gem_config::config::Imager::default();
    imager.remote_configs.push(existing_url);

    let new_config = Config {
        imager,
        os_list: vec![],
    };

    db.add_config(new_config, None)
        .expect("add_config should succeed");

    let updated_urls = db
        .remote_configs()
        .expect("Fetching remote configs should succeed");

    assert_eq!(
        updated_urls.len(),
        initial_count,
        "Duplicate remote config should not be inserted"
    );
}

#[test]
#[cfg_attr(
    not(feature = "sd"),
    ignore = "needs `sd`: fixture boards use Flasher::SdCard"
)]
fn add_config_inserts_device_into_board_list() {
    let db = Db::new().expect("Failed to create DB");

    db.init().expect("DB initialization should succeed");

    let initial_boards = db
        .board_list("")
        .expect("Fetching board list should succeed");

    let initial_count = initial_boards.len();

    let device = gem_config::config::Device {
        name: "Test Board".to_string(),
        tags: std::collections::HashSet::from(["test-board".to_string()]),
        icon: None,
        description: "Test device".to_string(),
        flasher: gem_config::config::Flasher::SdCard,
        emmc_dfu: false,
        documentation: None,
        instructions: None,
        specification: vec![],
        oshw: None,
    };

    let mut imager = gem_config::config::Imager::default();
    imager.devices.push(device.clone());

    let new_config = Config {
        imager,
        os_list: vec![],
    };

    db.add_config(new_config, None)
        .expect("add_config should succeed");

    let updated_boards = db
        .board_list("")
        .expect("Fetching board list should succeed");

    assert_eq!(
        updated_boards.len(),
        initial_count + 1,
        "One new device should be added to board_list"
    );

    assert!(
        updated_boards.iter().any(|b| b.name == "Test Board"),
        "Inserted device should appear in board_list"
    );
}

#[test]
#[cfg_attr(
    not(feature = "sd"),
    ignore = "needs `sd`: fixture boards use Flasher::SdCard"
)]
fn add_config_updates_existing_device_with_same_name() {
    let db = Db::new().expect("Failed to create DB");

    db.init().expect("DB initialization should succeed");

    let device_v1 = gem_config::config::Device {
        name: "Test Board".to_string(),
        tags: std::collections::HashSet::from(["test-board".to_string()]),
        icon: None,
        description: "Old description".to_string(),
        flasher: gem_config::config::Flasher::SdCard,
        emmc_dfu: false,
        documentation: None,
        instructions: None,
        specification: vec![],
        oshw: None,
    };

    let mut imager = gem_config::config::Imager::default();
    imager.devices.push(device_v1);

    db.add_config(
        Config {
            imager,
            os_list: vec![],
        },
        None,
    )
    .expect("First add_config should succeed");

    let boards = db
        .board_list("")
        .expect("Fetching board list should succeed");

    let board = boards
        .iter()
        .find(|b| b.name == "Test Board")
        .expect("Inserted board should exist");

    let board_id = board.id;
    let initial_count = boards.len();

    let device_v2 = gem_config::config::Device {
        name: "Test Board".to_string(),
        tags: std::collections::HashSet::from(["updated-tag".to_string()]),
        icon: None,
        description: "Updated description".to_string(),
        flasher: gem_config::config::Flasher::SdCard,
        emmc_dfu: false,
        documentation: None,
        instructions: Some("New instructions".to_string()),
        specification: vec![("CPU".to_string(), "Test CPU".to_string())],
        oshw: Some("us000000".to_string()),
    };

    let mut imager = gem_config::config::Imager::default();
    imager.devices.push(device_v2.clone());

    db.add_config(
        Config {
            imager,
            os_list: vec![],
        },
        None,
    )
    .expect("Second add_config should succeed");

    let updated_boards = db
        .board_list("")
        .expect("Fetching board list should succeed");

    assert_eq!(
        updated_boards.len(),
        initial_count,
        "Board with same name should be updated, not duplicated"
    );

    let updated_board = db
        .board_by_id(board_id)
        .expect("Fetching board by id should succeed");

    assert_eq!(updated_board.description, device_v2.description);
    assert_eq!(updated_board.flasher, device_v2.flasher);
    assert_eq!(updated_board.instructions, device_v2.instructions);
    assert_eq!(updated_board.oshw, device_v2.oshw);
    assert_eq!(updated_board.specification, device_v2.specification);
}

#[test]
#[cfg_attr(
    not(feature = "sd"),
    ignore = "needs `sd`: fixture boards use Flasher::SdCard"
)]
fn add_config_inserts_os_image_for_board() {
    let db = Db::new().expect("Failed to create DB");
    db.init().expect("DB init should succeed");

    let board = gem_config::config::Device {
        name: "Test Board".to_string(),
        description: "Test Board description".to_string(),
        icon: None,
        flasher: gem_config::config::Flasher::SdCard,
        instructions: None,
        oshw: None,
        specification: vec![],
        emmc_dfu: false,
        documentation: None,
        tags: HashSet::from(["test_board".to_string()]),
    };

    let image = gem_config::config::OsImage {
        name: "Test OS".to_string(),
        description: "Test OS description".to_string(),
        icon: "https://example.com/icon.png".try_into().unwrap(),
        url: "https://example.com/os.img.xz".try_into().unwrap(),
        image_download_size: Some(1024),
        image_download_sha256: [1; 32],
        extract_sha256: None,
        extract_size: 2048,
        release_date: chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        devices: HashSet::from(["test_board".to_string()]),
        tags: HashSet::new(),
        init_format: gem_config::config::InitFormat::None,
        info_text: None,
        support: None,
    };

    let config = Config {
        imager: gem_config::config::Imager {
            remote_configs: Default::default(),
            devices: vec![board.clone()],
        },
        os_list: vec![gem_config::config::OsListItem::Image(image.clone())],
    };

    db.add_config(config, None)
        .expect("add_config should succeed");

    let boards = db.board_list("").unwrap();
    let board_id = boards.iter().find(|b| b.name == board.name).unwrap().id;

    let items = db
        .os_image_items(board_id, None)
        .expect("os_image_items should succeed");

    assert!(
        items
            .iter()
            .any(|x| x.localized_label(gem_i18n::Lang::En) == image.name)
    );
}

#[test]
#[cfg_attr(
    not(feature = "sd"),
    ignore = "needs `sd`: fixture boards use Flasher::SdCard"
)]
fn os_image_by_id_returns_correct_data() {
    let db = Db::new().expect("Failed to create DB");
    db.init().expect("DB init should succeed");

    let board = gem_config::config::Device {
        name: "Test Board".to_string(),
        description: "Test Board description".to_string(),
        icon: None,
        flasher: gem_config::config::Flasher::SdCard,
        instructions: None,
        oshw: None,
        specification: vec![],
        emmc_dfu: false,
        documentation: None,
        tags: HashSet::from(["test_board".to_string()]),
    };

    let image = gem_config::config::OsImage {
        name: "Test OS".to_string(),
        description: "Test OS description".to_string(),
        icon: "https://example.com/icon.png".try_into().unwrap(),
        url: "https://example.com/os.img.xz".try_into().unwrap(),
        image_download_size: Some(1024),
        image_download_sha256: [7; 32],
        extract_sha256: None,
        extract_size: 4096,
        release_date: chrono::NaiveDate::from_ymd_opt(2024, 5, 10).unwrap(),
        devices: HashSet::from(["test_board".to_string()]),
        tags: HashSet::new(),
        init_format: gem_config::config::InitFormat::None,
        info_text: Some("Test info".to_string()),
        support: Some(
            "https://github.com/Toxpox/gem-imager-rs"
                .try_into()
                .unwrap(),
        ),
    };

    let config = Config {
        imager: gem_config::config::Imager {
            remote_configs: Default::default(),
            devices: vec![board],
        },
        os_list: vec![gem_config::config::OsListItem::Image(image.clone())],
    };

    db.add_config(config, None)
        .expect("add_config should succeed");

    let boards = db.board_list("").unwrap();
    let board_id = boards.iter().find(|b| b.name == "Test Board").unwrap().id;

    let items = db
        .os_image_items(board_id, None)
        .expect("os_image_items should succeed");

    let crate::helpers::OsImageId::OsImage(image_id) = items
        .iter()
        .find(|x| x.localized_label(gem_i18n::Lang::En) == "Test OS")
        .unwrap()
        .id
    else {
        panic!("Incorrect ID");
    };
    let stored = db
        .os_image_by_id(image_id)
        .expect("os_image_by_id should succeed");

    assert_eq!(stored.name, image.name);
    assert_eq!(stored.description, image.description);
    assert_eq!(stored.url.as_str(), image.url.as_str());
    assert_eq!(stored.icon.as_str(), image.icon.as_str());
    assert_eq!(stored.image_download_size, Some(1024));
    assert_eq!(stored.image_download_sha256, [7; 32]);
    assert_eq!(stored.extract_size, 4096);
    assert_eq!(stored.release_date, image.release_date);
    assert_eq!(stored.init_format, image.init_format);
    assert_eq!(stored.info_text, image.info_text);
}

#[test]
#[cfg_attr(
    not(feature = "sd"),
    ignore = "needs `sd`: fixture boards use Flasher::SdCard"
)]
fn add_config_inserts_os_sublist_for_board() {
    let db = Db::new().expect("Failed to create DB");
    db.init().expect("DB init should succeed");

    let board = gem_config::config::Device {
        name: "Test Board".to_string(),
        description: "Test Board description".to_string(),
        icon: None,
        flasher: gem_config::config::Flasher::SdCard,
        instructions: None,
        oshw: None,
        specification: vec![],
        emmc_dfu: false,
        documentation: None,
        tags: HashSet::from(["test_board".to_string()]),
    };

    let image = gem_config::config::OsImage {
        name: "Test OS".to_string(),
        description: "Test OS description".to_string(),
        icon: "https://example.com/icon.png".try_into().unwrap(),
        url: "https://example.com/os.img.xz".try_into().unwrap(),
        image_download_size: Some(1024),
        image_download_sha256: [1; 32],
        extract_sha256: None,
        extract_size: 2048,
        release_date: chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        devices: HashSet::from(["test_board".to_string()]),
        tags: HashSet::new(),
        init_format: gem_config::config::InitFormat::None,
        info_text: None,
        support: None,
    };

    let sublist = gem_config::config::OsSubList {
        name: "Test SubList".to_string(),
        description: "SubList description".to_string(),
        icon: "https://example.com/sublist.png".try_into().unwrap(),
        flasher: gem_config::config::Flasher::SdCard,
        subitems: vec![gem_config::config::OsListItem::Image(image)],
    };

    let config = Config {
        imager: gem_config::config::Imager {
            remote_configs: Default::default(),
            devices: vec![board],
        },
        os_list: vec![gem_config::config::OsListItem::SubList(sublist)],
    };

    db.add_config(config, None)
        .expect("add_config should succeed");

    let boards = db.board_list("").unwrap();
    let board_id = boards.iter().find(|b| b.name == "Test Board").unwrap().id;

    let items = db
        .os_image_items(board_id, None)
        .expect("os_image_items should succeed");

    assert!(
        items
            .iter()
            .any(|x| x.localized_label(gem_i18n::Lang::En) == "Test SubList")
    );
}

#[test]
#[cfg_attr(
    not(feature = "sd"),
    ignore = "needs `sd`: fixture boards use Flasher::SdCard"
)]
fn nested_os_sublists_propagate_board_support() {
    let db = Db::new().expect("Failed to create DB");
    db.init().expect("DB init should succeed");

    let board = gem_config::config::Device {
        name: "Test Board".to_string(),
        description: "Test Board description".to_string(),
        icon: None,
        flasher: gem_config::config::Flasher::SdCard,
        instructions: None,
        oshw: None,
        specification: vec![],
        emmc_dfu: false,
        documentation: None,
        tags: HashSet::from(["test_board".to_string()]),
    };

    let image = gem_config::config::OsImage {
        name: "Nested OS".to_string(),
        description: "Nested OS description".to_string(),
        icon: "https://example.com/icon.png".try_into().unwrap(),
        url: "https://example.com/os.img.xz".try_into().unwrap(),
        image_download_size: Some(1024),
        image_download_sha256: [1; 32],
        extract_sha256: None,
        extract_size: 2048,
        release_date: chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        devices: HashSet::from(["test_board".to_string()]),
        tags: HashSet::new(),
        init_format: gem_config::config::InitFormat::None,
        info_text: None,
        support: None,
    };

    let child_sublist = gem_config::config::OsSubList {
        name: "Child SubList".to_string(),
        description: "Child description".to_string(),
        icon: "https://example.com/child.png".try_into().unwrap(),
        flasher: gem_config::config::Flasher::SdCard,
        subitems: vec![gem_config::config::OsListItem::Image(image)],
    };

    let parent_sublist = gem_config::config::OsSubList {
        name: "Parent SubList".to_string(),
        description: "Parent description".to_string(),
        icon: "https://example.com/parent.png".try_into().unwrap(),
        flasher: gem_config::config::Flasher::SdCard,
        subitems: vec![gem_config::config::OsListItem::SubList(child_sublist)],
    };

    let config = Config {
        imager: gem_config::config::Imager {
            remote_configs: Default::default(),
            devices: vec![board],
        },
        os_list: vec![gem_config::config::OsListItem::SubList(parent_sublist)],
    };

    db.add_config(config, None)
        .expect("add_config should succeed");

    let board_id = db
        .board_list("")
        .unwrap()
        .into_iter()
        .find(|b| b.name == "Test Board")
        .unwrap()
        .id;

    let items = db
        .os_image_items(board_id, None)
        .expect("os_image_items should succeed");

    assert!(
        items
            .iter()
            .any(|x| x.localized_label(gem_i18n::Lang::En) == "Parent SubList"),
        "Parent sublist should be visible through recursive propagation"
    );
}

#[test]
#[cfg_attr(
    not(feature = "sd"),
    ignore = "needs `sd`: fixture boards use Flasher::SdCard"
)]
fn remote_os_sublist_is_returned_for_board() {
    let db = Db::new().expect("Failed to create DB");
    db.init().expect("DB init should succeed");

    let board = gem_config::config::Device {
        name: "Test Board".to_string(),
        description: "Test Board description".to_string(),
        icon: None,
        flasher: gem_config::config::Flasher::SdCard,
        instructions: None,
        oshw: None,
        specification: vec![],
        emmc_dfu: false,
        documentation: None,
        tags: HashSet::from(["test_board".to_string()]),
    };

    let remote_sublist = gem_config::config::OsRemoteSubList {
        name: "Remote OS List".to_string(),
        description: "Remote description".to_string(),
        icon: "https://example.com/remote.png".try_into().unwrap(),
        flasher: gem_config::config::Flasher::SdCard,
        subitems_url: "https://example.com/os-list.json".try_into().unwrap(),
        devices: HashSet::from(["test_board".to_string()]),
    };

    let config = Config {
        imager: gem_config::config::Imager {
            remote_configs: Default::default(),
            devices: vec![board],
        },
        os_list: vec![gem_config::config::OsListItem::RemoteSubList(
            remote_sublist,
        )],
    };

    db.add_config(config, None)
        .expect("add_config should succeed");

    let board_id = db
        .board_list("")
        .unwrap()
        .into_iter()
        .find(|b| b.name == "Test Board")
        .unwrap()
        .id;

    let remote_lists = db
        .os_remote_sublists(board_id, None)
        .expect("os_remote_sublists should succeed");

    assert_eq!(remote_lists.len(), 1);
    assert_eq!(
        remote_lists[0].1.as_str(),
        "https://example.com/os-list.json"
    );
}

#[test]
#[cfg_attr(
    not(feature = "sd"),
    ignore = "needs `sd`: fixture boards use Flasher::SdCard"
)]
fn remote_os_sublist_resolve_inserts_child_items_and_clears_url() {
    let db = Db::new().expect("Failed to create DB");
    db.init().expect("DB init should succeed");

    let board = gem_config::config::Device {
        name: "Test Board".to_string(),
        description: "Test Board description".to_string(),
        icon: None,
        flasher: gem_config::config::Flasher::SdCard,
        instructions: None,
        oshw: None,
        specification: vec![],
        emmc_dfu: false,
        documentation: None,
        tags: HashSet::from(["test_board".to_string()]),
    };

    let remote_sublist = gem_config::config::OsRemoteSubList {
        name: "Remote OS List".to_string(),
        description: "Remote description".to_string(),
        icon: "https://example.com/remote.png".try_into().unwrap(),
        flasher: gem_config::config::Flasher::SdCard,
        subitems_url: "https://example.com/os-list.json".try_into().unwrap(),
        devices: HashSet::from(["test_board".to_string()]),
    };

    let config = Config {
        imager: gem_config::config::Imager {
            remote_configs: Default::default(),
            devices: vec![board],
        },
        os_list: vec![gem_config::config::OsListItem::RemoteSubList(
            remote_sublist,
        )],
    };

    db.add_config(config, None)
        .expect("add_config should succeed");

    let board_id = db
        .board_list("")
        .unwrap()
        .into_iter()
        .find(|b| b.name == "Test Board")
        .unwrap()
        .id;

    let remote_lists = db.os_remote_sublists(board_id, None).unwrap();

    assert_eq!(remote_lists.len(), 1);

    let sublist_id = remote_lists[0].0;

    let child_image = gem_config::config::OsImage {
        name: "Fetched OS".to_string(),
        description: "Fetched OS description".to_string(),
        icon: "https://example.com/icon.png".try_into().unwrap(),
        url: "https://example.com/os.img.xz".try_into().unwrap(),
        image_download_size: Some(1024),
        image_download_sha256: [1; 32],
        extract_sha256: None,
        extract_size: 2048,
        release_date: chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        devices: HashSet::from(["test_board".to_string()]),
        tags: HashSet::new(),
        init_format: gem_config::config::InitFormat::None,
        info_text: None,
        support: None,
    };

    db.os_remote_sublist_resolve(
        sublist_id,
        &[gem_config::config::OsListItem::Image(child_image)],
    )
    .expect("resolve should succeed");

    let remote_lists_after = db.os_remote_sublists(board_id, None).unwrap();

    assert!(remote_lists_after.is_empty(),);

    let items = db.os_image_items(board_id, Some(sublist_id)).unwrap();

    assert!(
        items
            .iter()
            .any(|x| x.localized_label(gem_i18n::Lang::En) == "Fetched OS"),
    );
}

#[test]
#[cfg_attr(
    not(feature = "sd"),
    ignore = "needs `sd`: fixture boards use Flasher::SdCard"
)]
fn duplicate_remote_sublist_resolve_does_not_duplicate_os_items() {
    let db = Db::new().expect("Failed to create DB");
    db.init().expect("DB init should succeed");

    let board = gem_config::config::Device {
        name: "Test Board".to_string(),
        description: "Test Board description".to_string(),
        icon: None,
        flasher: gem_config::config::Flasher::SdCard,
        instructions: None,
        oshw: None,
        specification: vec![],
        emmc_dfu: false,
        documentation: None,
        tags: HashSet::from(["test_board".to_string()]),
    };

    let remote_sublist = gem_config::config::OsRemoteSubList {
        name: "Remote OS List".to_string(),
        description: "Remote description".to_string(),
        icon: "https://example.com/remote.png".try_into().unwrap(),
        flasher: gem_config::config::Flasher::SdCard,
        subitems_url: "https://example.com/os-list.json".try_into().unwrap(),
        devices: HashSet::from(["test_board".to_string()]),
    };

    let config = Config {
        imager: gem_config::config::Imager {
            remote_configs: Default::default(),
            devices: vec![board],
        },
        os_list: vec![gem_config::config::OsListItem::RemoteSubList(
            remote_sublist,
        )],
    };

    db.add_config(config, None)
        .expect("add_config should succeed");

    let board_id = db
        .board_list("")
        .unwrap()
        .into_iter()
        .find(|b| b.name == "Test Board")
        .unwrap()
        .id;

    let remote_lists = db.os_remote_sublists(board_id, None).unwrap();

    assert_eq!(remote_lists.len(), 1);

    let sublist_id = remote_lists[0].0;

    let child_image = gem_config::config::OsImage {
        name: "Fetched OS".to_string(),
        description: "Fetched OS description".to_string(),
        icon: "https://example.com/icon.png".try_into().unwrap(),
        url: "https://example.com/os.img.xz".try_into().unwrap(),
        image_download_size: Some(1024),
        image_download_sha256: [1; 32],
        extract_sha256: None,
        extract_size: 2048,
        release_date: chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        devices: HashSet::from(["test_board".to_string()]),
        tags: HashSet::new(),
        init_format: gem_config::config::InitFormat::None,
        info_text: None,
        support: None,
    };

    db.os_remote_sublist_resolve(
        sublist_id,
        &[gem_config::config::OsListItem::Image(child_image.clone())],
    )
    .expect("first resolve should succeed");

    db.os_remote_sublist_resolve(
        sublist_id,
        &[gem_config::config::OsListItem::Image(child_image)],
    )
    .expect("a repeated resolve must not abort the merge");

    let items = db.os_image_items(board_id, Some(sublist_id)).unwrap();
    let count = items
        .iter()
        .filter(|x| x.localized_label(gem_i18n::Lang::En) == "Fetched OS")
        .count();

    assert_eq!(count, 1,);

    let remote_lists_after = db.os_remote_sublists(board_id, None).unwrap();

    assert!(remote_lists_after.is_empty(),);
}

#[test]
#[cfg_attr(
    not(feature = "sd"),
    ignore = "needs `sd`: fixture boards use Flasher::SdCard"
)]
fn board_list_search_filters_boards_case_insensitive() {
    let db = Db::new().expect("Failed to create DB");
    db.init().expect("DB init should succeed");

    let board1 = gem_config::config::Device {
        name: "Test Board 1".to_string(),
        description: "Board 1".to_string(),
        icon: None,
        flasher: gem_config::config::Flasher::SdCard,
        instructions: None,
        oshw: None,
        specification: vec![],
        emmc_dfu: false,
        documentation: None,
        tags: HashSet::from(["bbb".to_string()]),
    };

    let board2 = gem_config::config::Device {
        name: "Test Board 2".to_string(),
        description: "Board 2".to_string(),
        icon: None,
        flasher: gem_config::config::Flasher::SdCard,
        instructions: None,
        oshw: None,
        specification: vec![],
        emmc_dfu: false,
        documentation: None,
        tags: HashSet::from(["beagleplay".to_string()]),
    };

    let board3 = gem_config::config::Device {
        name: "Test Board 3".to_string(),
        description: "Board 3".to_string(),
        icon: None,
        flasher: gem_config::config::Flasher::SdCard,
        instructions: None,
        oshw: None,
        specification: vec![],
        emmc_dfu: false,
        documentation: None,
        tags: HashSet::from(["rpi".to_string()]),
    };

    let config = Config {
        imager: gem_config::config::Imager {
            remote_configs: Default::default(),
            devices: vec![board1, board2, board3],
        },
        os_list: vec![],
    };

    db.add_config(config, None)
        .expect("add_config should succeed");

    let results = db.board_list("test").expect("search should succeed");

    assert_eq!(
        results.len(),
        3,
        "Only boards containing 'test' should be returned"
    );

    assert!(results.iter().any(|b| b.name == "Test Board 1"));
    assert!(results.iter().any(|b| b.name == "Test Board 2"));
    assert!(results.iter().any(|b| b.name == "Test Board 3"));
}

#[test]
#[cfg_attr(
    not(feature = "sd"),
    ignore = "needs `sd`: fixture boards use Flasher::SdCard"
)]
fn board_rows_carry_catalog_tags() {
    let db = Db::new().expect("Failed to create DB");
    db.init().expect("DB init should succeed");

    let board = gem_config::config::Device {
        name: "T3-GEM-O1".to_string(),
        description: "T3 Gemstone Obsidian".to_string(),
        icon: None,
        flasher: gem_config::config::Flasher::SdCard,
        instructions: None,
        oshw: None,
        specification: vec![],
        emmc_dfu: true,
        documentation: None,
        tags: HashSet::from(["t3-gem-o1".to_string(), "extra-tag".to_string()]),
    };

    let config = Config {
        imager: gem_config::config::Imager {
            remote_configs: Default::default(),
            devices: vec![board],
        },
        os_list: vec![],
    };

    db.add_config(config, None)
        .expect("add_config should succeed");

    let row = db
        .board_list("T3-GEM-O1")
        .expect("search should succeed")
        .into_iter()
        .find(|b| b.name == "T3-GEM-O1")
        .expect("the inserted board should be listed");

    assert!(
        row.tags.iter().any(|t| t == "t3-gem-o1"),
        "board_list() must carry the catalog tag the board photo is matched on, got {:?}",
        row.tags
    );
    assert_eq!(
        row.tags.len(),
        2,
        "both tags should survive, got {:?}",
        row.tags
    );

    let detail = db.board_by_id(row.id).expect("board_by_id should succeed");
    assert!(
        detail.tags.iter().any(|t| t == "t3-gem-o1"),
        "board_by_id() must carry the catalog tag, got {:?}",
        detail.tags
    );
}

#[test]
#[cfg_attr(
    not(feature = "sd"),
    ignore = "needs `sd`: fixture boards use Flasher::SdCard"
)]
fn an_untagged_board_reports_no_tags() {
    let db = Db::new().expect("Failed to create DB");
    db.init().expect("DB init should succeed");

    let board = gem_config::config::Device {
        name: "No filtering".to_string(),
        description: "Show every possible image".to_string(),
        icon: None,
        flasher: gem_config::config::Flasher::SdCard,
        instructions: None,
        oshw: None,
        specification: vec![],
        emmc_dfu: false,
        documentation: None,
        tags: HashSet::new(),
    };

    let config = Config {
        imager: gem_config::config::Imager {
            remote_configs: Default::default(),
            devices: vec![board],
        },
        os_list: vec![],
    };

    db.add_config(config, None)
        .expect("add_config should succeed");

    let row = db
        .board_list("No filtering")
        .expect("search should succeed")
        .into_iter()
        .find(|b| b.name == "No filtering")
        .expect("the inserted board should be listed");

    assert!(
        row.tags.is_empty(),
        "an untagged board must report no tags, got {:?}",
        row.tags
    );
}

#[test]
#[cfg_attr(
    not(feature = "sd"),
    ignore = "needs `sd`: fixture boards use Flasher::SdCard"
)]
fn copied_board_json_round_trips_as_a_catalog_entry() {
    let device = gem_config::config::Device {
        name: "Test Board".to_string(),
        tags: std::collections::HashSet::from(["test-board".to_string()]),
        icon: None,
        description: "A board".to_string(),
        flasher: gem_config::config::Flasher::SdCard,
        emmc_dfu: false,
        documentation: None,
        instructions: None,
        specification: vec![],
        oshw: None,
    };

    let db = Db::new().expect("Failed to create DB");
    db.init().expect("DB initialization should succeed");

    let mut imager = gem_config::config::Imager::default();
    imager.devices.push(device);
    db.add_config(
        Config {
            imager,
            os_list: vec![],
        },
        None,
    )
    .expect("add_config should succeed");

    let board_id = db
        .board_list("")
        .expect("board list")
        .iter()
        .find(|b| b.name == "Test Board")
        .expect("inserted board exists")
        .id;
    let board = db.board_by_id(board_id).expect("board by id");

    let entry = gem_config::config::Device::from(&board);
    let json = serde_json::to_string_pretty(&entry).expect("board serializes");

    let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert!(
        value.get("id").is_none(),
        "the SQLite rowid leaks into the clipboard and is meaningless outside this database:\n{json}"
    );

    serde_json::from_str::<gem_config::config::Device>(&json)
        .expect("copied JSON should deserialize as a catalog device entry");
}
