
use iced::Task;

use crate::{
    GemImager,
    helpers::{self, blocking_future},
    state::{OverlayData, OverlayState},
};

#[derive(Debug, Clone)]
pub(crate) enum GemImagerMessage {
    Null,

    ExtendConfig((i64, gem_config::Config)),
    ResolveRemoteSubitemItem {
        item: Vec<gem_config::config::OsListItem>,
        target: i64,
    },

    UpdateAvailable(semver::Version),

    UpdateBoardList(Vec<crate::db::BoardListItem>),
    SelectBoardById(i64),
    SelectBoard(crate::db::Board),

    UpdateOsList((Vec<helpers::OsImageItem>, Option<i64>)),
    SelectOs(helpers::OsImageId),
    SelectLocalOs(helpers::BoardImage),
    SelectRemoteOs((crate::db::OsImage, gem_config::config::Flasher)),
    GotoOsListParent,
    UpdateInitFormat(gem_config::config::InitFormat),

    SelectDest(helpers::Destination),
    SelectFileDest(String),
    DestinationFilter(bool),
    ShowDfuNotReadyNotice,
    DismissNotice,

    UpdateFlashConfig(crate::helpers::FlashingCustomization),
    ResetFlashingConfig,
    ToggleWifi(bool),
    WifiAutofill(crate::helpers::HostWifiPrefill),

    RequestFlash,
    CancelFlashRequest,
    FlashStart,

    SetLanguage(gem_i18n::Lang),

    FlashProgress(gem_flasher::DownloadFlashingStatus),
    FlashSuccess,
    FlashCancel,
    FlashFail(String),

    Restart,
    Retry,

    OpenUrl(url::Url),

    Next,
    Back,

    ResolveImage(url::Url, std::path::PathBuf),
    FilterResolveImages(Vec<url::Url>),

    Destinations(Vec<helpers::Destination>),

    #[cfg(feature = "dfu-driver-mvp")]
    DfuDriverProbe(gem_winusb::DriverState),
    #[cfg(feature = "dfu-driver-mvp")]
    DfuDriverInstall,
    #[cfg(feature = "dfu-driver-mvp")]
    DfuDriverInstallFinished(Result<gem_winusb::InstallOutcome, gem_winusb::InstallError>),
    #[cfg(feature = "dfu-driver-mvp")]
    DfuDriverDismiss,
    #[cfg(feature = "dfu-driver-mvp")]
    DfuDriverShowDetails,
    #[cfg(feature = "dfu-driver-mvp")]
    DfuDriverBackToOffer,

    EditorEvent(iced::widget::text_editor::Action),

    AppInfo,

    CopyToClipboard(String),

    DbInitSuccess,

    UpdateSearchText(String),
}

pub(crate) fn update(state: &mut GemImager, message: GemImagerMessage) -> Task<GemImagerMessage> {
    match message {
        GemImagerMessage::SetLanguage(lang) => state.common_mut().set_lang(lang),
        GemImagerMessage::RequestFlash => match state {
            GemImager::Review(inner) => inner.erase_confirmation = true,
            _ => panic!("Unexpected message"),
        },
        GemImagerMessage::CancelFlashRequest => match state {
            GemImager::Review(inner) => inner.erase_confirmation = false,
            _ => panic!("Unexpected message"),
        },
        GemImagerMessage::SelectBoardById(id) => {
            let db = state.common().db.clone();
            return Task::perform(
                blocking_future(move || db.board_by_id(id).expect("Incorrect board id")),
                GemImagerMessage::SelectBoard,
            );
        }
        GemImagerMessage::UpdateBoardList(boards) => {
            match state {
                GemImager::ChooseBoard(x) => {
                    x.boards = boards;
                }
                GemImager::AppInfo(overlay_state) => {
                    if let OverlayData::ChooseBoard(x) = &mut overlay_state.page {
                        x.boards = boards;
                    }
                }
                _ => {}
            }
        }
        GemImagerMessage::SelectBoard(b) => match state {
            GemImager::ChooseBoard(inner) => {
                inner.selected_board = Some(b);
            }
            GemImager::AppInfo(overlay_state) => {
                if let OverlayData::ChooseBoard(inner) = &mut overlay_state.page {
                    inner.selected_board = Some(b);
                }
            }
            _ => {}
        },
        GemImagerMessage::UpdateOsList((imgs, pos)) => {
            match state {
                GemImager::ChooseOs(inner) => inner.update_images(imgs, pos),
                GemImager::AppInfo(overlay_state) => {
                    if let OverlayData::ChooseOs(inner) = &mut overlay_state.page {
                        inner.update_images(imgs, pos)
                    }
                }
                _ => {}
            };
        }
        GemImagerMessage::SelectOs(id) => match state {
            GemImager::ChooseOs(inner) => match id {
                helpers::OsImageId::Format => {
                    inner.selected_image = Some((id, helpers::BoardImage::format()))
                }
                helpers::OsImageId::Local(flasher) => {
                    let extensions = helpers::file_filter(flasher);

                    return Task::perform(
                        async move {
                            rfd::AsyncFileDialog::new()
                                .add_filter("image", extensions)
                                .pick_file()
                                .await
                                .map(|x| x.inner().to_path_buf())
                        },
                        move |x| match x {
                            Some(y) => GemImagerMessage::SelectLocalOs(helpers::BoardImage::local(
                                y, flasher,
                            )),
                            None => GemImagerMessage::Null,
                        },
                    );
                }
                helpers::OsImageId::OsImage(id) => {
                    let db = inner.common.db.clone();
                    let flasher = inner.flasher;
                    return Task::perform(
                        blocking_future(move || db.os_image_by_id(id)),
                        move |x| match x {
                            Ok(i) => GemImagerMessage::SelectRemoteOs((i, flasher)),
                            Err(e) => {
                                tracing::error!("Failed to get os image {e}");
                                GemImagerMessage::Null
                            }
                        },
                    );
                }
                helpers::OsImageId::OsSublist(id) => {
                    let board_id = inner.selected_board.id;
                    return Task::batch([
                        inner.resolve_remote_sublists(board_id, Some(id.0)),
                        inner.update_pos(Some(id.0), id.1),
                    ]);
                }
            },
            _ => panic!("Unexpected message"),
        },
        GemImagerMessage::SelectRemoteOs((image, flasher)) => match state {
            GemImager::ChooseOs(inner) => {
                inner.selected_image = Some((
                    helpers::OsImageId::OsImage(image.id),
                    helpers::BoardImage::remote(image, flasher, inner.common.downloader.clone()),
                ));
            }
            GemImager::AppInfo(overlay_state) => {
                if let OverlayData::ChooseOs(inner) = &mut overlay_state.page {
                    inner.selected_image = Some((
                        helpers::OsImageId::OsImage(image.id),
                        helpers::BoardImage::remote(
                            image,
                            flasher,
                            inner.common.downloader.clone(),
                        ),
                    ));
                }
            }
            _ => {}
        },
        GemImagerMessage::SelectLocalOs(image) => {
            if let GemImager::ChooseOs(inner) = state {
                inner.selected_image = Some((helpers::OsImageId::Local(image.flasher()), image))
            }
        }
        GemImagerMessage::OpenUrl(x) => {
            return Task::future(async move {
                let res = webbrowser::open(x.as_str());
                tracing::debug!("Open Url Resp {res:?}");
                GemImagerMessage::Null
            });
        }
        GemImagerMessage::Next => return state.next(),
        GemImagerMessage::Back => return state.back(),
        GemImagerMessage::ResolveImage(k, v) => state.image_cache_insert(k, v),
        GemImagerMessage::FilterResolveImages(x) => {
            let common = state.common_mut();
            let iter = x.into_iter().filter(|x| {
                if common.img_handle_cache.contains(x) {
                    false
                } else {
                    common.img_handle_cache.mark_fetching(x.clone());
                    true
                }
            });
            return helpers::fetch_images(&common.downloader, iter);
        }
        GemImagerMessage::ExtendConfig((u, c)) => {
            tracing::debug!("Update Config: {:#?}", c);

            let db = state.common().db.clone();
            let db_task = Task::perform(blocking_future(move || db.add_config(c, Some(u))), |x| {
                if let Err(e) = x {
                    tracing::error!("Failed to merge config {e}");
                }
                GemImagerMessage::Null
            });

            let tail_tasks = match state {
                GemImager::ChooseBoard(inner) => Task::batch([
                    inner.common.fetch_board_images(),
                    inner.refresh_board_list(),
                ]),
                GemImager::ChooseOs(inner) => {
                    let board_id = inner.selected_board.id;
                    let db = inner.common.db.clone();
                    let downloader = inner.common.downloader.clone();

                    let remote_items_fetch = Task::future(blocking_future(move || {
                        db.os_remote_sublists_by_remote_config(board_id, u).unwrap()
                    }))
                    .then(move |items| {
                        let dl = downloader.clone();
                        helpers::fetch_remote_subitems(items, dl)
                    });

                    Task::batch([inner.common.fetch_board_images(), remote_items_fetch])
                }
                _ => state.common().fetch_board_images(),
            };

            return db_task.chain(tail_tasks);
        }
        GemImagerMessage::ResolveRemoteSubitemItem { item, target } => {
            let db = state.common().db.clone();
            let tail = match &state {
                GemImager::ChooseOs(inner) => Task::batch([
                    inner.resolve_remote_sublists(inner.selected_board.id, Some(target)),
                    inner.refresh_image_list(),
                    state.refresh_image_icons(inner.selected_board.id),
                ]),
                _ => Task::none(),
            };

            return Task::future(blocking_future(move || {
                db.os_remote_sublist_resolve(target, &item).unwrap();
                GemImagerMessage::Null
            }))
            .chain(tail);
        }
        GemImagerMessage::UpdateAvailable(x) => {
            return show_notification(gem_i18n::fmt::update_available(
                state.common().lang(),
                &x.to_string(),
            ));
        }
        GemImagerMessage::GotoOsListParent => match state {
            GemImager::ChooseOs(inner) => {
                let db = inner.common.db.clone();
                let curpos = inner.pos.unwrap();
                let board_id = inner.selected_board.id;
                return Task::perform(
                    blocking_future(move || {
                        let id = db.os_sublist_parent(curpos).unwrap();
                        let imgs = db.os_image_items(board_id, id).unwrap();
                        (imgs, id)
                    }),
                    GemImagerMessage::UpdateOsList,
                );
            }
            _ => panic!("Unexpected message"),
        },
        GemImagerMessage::Destinations(x) => {
            if let GemImager::ChooseDest(inner) = state
                && x != inner.destinations
            {
                inner.selected_dest =
                    helpers::keep_selected_destination(inner.selected_dest.take(), &x);
                inner.dfu_notice &= !x.iter().any(|d| d.is_dfu());
                inner.destinations = x;
            }
        }
        #[cfg(feature = "dfu-driver-mvp")]
        GemImagerMessage::DfuDriverProbe(driver_state) => {
            state.common_mut().dfu_driver.on_probe(driver_state);
        }
        #[cfg(feature = "dfu-driver-mvp")]
        GemImagerMessage::DfuDriverInstall => {
            if state.common_mut().dfu_driver.begin_install() {
                return crate::driver_ui::install_task();
            }
        }
        #[cfg(feature = "dfu-driver-mvp")]
        GemImagerMessage::DfuDriverInstallFinished(result) => {
            state.common_mut().dfu_driver.finish_install(result);
        }
        #[cfg(feature = "dfu-driver-mvp")]
        GemImagerMessage::DfuDriverDismiss => {
            state.common_mut().dfu_driver.dismiss();
        }
        #[cfg(feature = "dfu-driver-mvp")]
        GemImagerMessage::DfuDriverShowDetails => {
            state.common_mut().dfu_driver.show_details();
        }
        #[cfg(feature = "dfu-driver-mvp")]
        GemImagerMessage::DfuDriverBackToOffer => {
            state.common_mut().dfu_driver.back_to_offer();
        }
        GemImagerMessage::SelectDest(x) => match state {
            GemImager::ChooseDest(inner) => {
                inner.selected_dest = Some(x);
            }
            _ => panic!("Unexpected message"),
        },
        GemImagerMessage::ShowDfuNotReadyNotice => {
            if let GemImager::ChooseDest(inner) = state {
                inner.dfu_notice = true;
            }
        }
        GemImagerMessage::DismissNotice => match state {
            GemImager::ChooseDest(inner) => inner.dfu_notice = false,
            GemImager::FlashingSuccess(inner) => inner.notice_dismissed = true,
            _ => {}
        },
        GemImagerMessage::SelectFileDest(x) => {
            return Task::perform(
                async move {
                    rfd::AsyncFileDialog::new()
                        .set_file_name(x)
                        .save_file()
                        .await
                        .map(|x| x.inner().to_path_buf())
                },
                move |x| match x {
                    Some(y) => GemImagerMessage::SelectDest(helpers::Destination::LocalFile(y)),
                    None => GemImagerMessage::Null,
                },
            );
        }
        GemImagerMessage::DestinationFilter(x) => match state {
            GemImager::ChooseDest(inner) => {
                inner.filter_destination = x;
            }
            _ => panic!("Unexpected message"),
        },
        GemImagerMessage::UpdateFlashConfig(x) => match state {
            GemImager::Customize(inner) => {
                inner.customization = x;
            }
            _ => panic!("Unexpected message"),
        },
        GemImagerMessage::WifiAutofill(prefill) => {
            if let GemImager::Customize(inner) = state {
                inner.customization.apply_wifi_prefill(prefill);
            }
        }
        GemImagerMessage::ToggleWifi(enabled) => match state {
            GemImager::Customize(inner) => {
                if enabled {
                    inner.customization.enable_wifi();
                    return Task::perform(
                        blocking_future(helpers::detect_host_wifi),
                        GemImagerMessage::WifiAutofill,
                    );
                }
                inner.customization.disable_wifi();
            }
            _ => panic!("Unexpected message"),
        },
        GemImagerMessage::ResetFlashingConfig => match state {
            GemImager::Customize(inner) => {
                inner.customization.reset();
            }
            _ => panic!("Unexpected message"),
        },
        GemImagerMessage::FlashCancel => {
            let lang = state.common().lang();
            let mut msg = lang.text(gem_i18n::Msg::FlashCancelledNotification);

            *state = match std::mem::take(state) {
                GemImager::Flashing(inner) => {
                    inner.cancel_flashing.abort();

                    if inner.is_download {
                        msg = lang.text(gem_i18n::Msg::DownloadCancelledNotification);
                    }
                    GemImager::FlashingCancel(inner.into())
                }
                GemImager::AppInfo(inner) => match inner.page {
                    OverlayData::Flashing(flashing_state) => {
                        flashing_state.cancel_flashing.abort();

                        if flashing_state.is_download {
                            msg = lang.text(gem_i18n::Msg::DownloadCancelledNotification);
                        }

                        GemImager::AppInfo(OverlayState {
                            page: OverlayData::FlashingCancel(flashing_state.into()),
                            ..inner
                        })
                    }
                    _ => panic!("Unexpected message"),
                },
                _ => panic!("Unexpected message"),
            };

            return show_notification(msg.to_string());
        }
        GemImagerMessage::Restart => {
            return state.restart();
        }
        GemImagerMessage::FlashFail(err) => {
            let lang = state.common().lang();
            let user_err = localized_flash_error(lang, &err);
            let mut msg = lang.text(gem_i18n::Msg::FlashFailedNotification);

            let logs =
                std::fs::read_to_string(helpers::log_file_path()).expect("Failed to read logs");
            let logs = iced::widget::text_editor::Content::with_text(&logs);

            *state = match std::mem::take(state) {
                GemImager::Flashing(inner) => {
                    if inner.is_download {
                        msg = lang.text(gem_i18n::Msg::DownloadFailedNotification);
                    }

                    GemImager::FlashingFail(crate::state::FlashingFailState {
                        common: inner.common,
                        err: user_err.clone(),
                        logs,
                        selected_board: inner.selected_board,
                        selected_image: inner.selected_image,
                        selected_dest: inner.selected_dest,
                        customization: inner.customization,
                    })
                }
                GemImager::AppInfo(inner) => match inner.page {
                    OverlayData::Flashing(flashing_state) => {
                        if flashing_state.is_download {
                            msg = lang.text(gem_i18n::Msg::DownloadFailedNotification);
                        }

                        GemImager::AppInfo(OverlayState {
                            page: OverlayData::FlashingFail(crate::state::FlashingFailState {
                                common: flashing_state.common,
                                err: user_err,
                                logs,
                                selected_board: flashing_state.selected_board,
                                selected_image: flashing_state.selected_image,
                                selected_dest: flashing_state.selected_dest,
                                customization: flashing_state.customization,
                            }),
                            ..inner
                        })
                    }
                    _ => panic!("Unexpected message"),
                },
                _ => panic!("Unexpected message"),
            };

            return show_notification(msg.to_string());
        }
        GemImagerMessage::FlashProgress(x) => match state {
            GemImager::Flashing(inner) => {
                inner.progress_update(x);
            }
            GemImager::AppInfo(inner) => match &mut inner.page {
                OverlayData::Flashing(flashing_state) => flashing_state.progress_update(x),
                _ => panic!("Unexpected message"),
            },
            _ => {}
        },
        GemImagerMessage::FlashStart | GemImagerMessage::Retry => {
            return state.start_flashing();
        }
        GemImagerMessage::FlashSuccess => {
            let lang = state.common().lang();
            let mut msg = lang.text(gem_i18n::Msg::FlashFinishedNotification);

            *state = match std::mem::take(state) {
                GemImager::Flashing(inner) => {
                    if inner.is_download {
                        msg = lang.text(gem_i18n::Msg::DownloadFinishedNotification);
                    }
                    GemImager::FlashingSuccess(inner.into())
                }
                GemImager::AppInfo(inner) => match inner.page {
                    OverlayData::Flashing(flashing_state) => {
                        if flashing_state.is_download {
                            msg = lang.text(gem_i18n::Msg::DownloadFinishedNotification);
                        }

                        GemImager::AppInfo(OverlayState {
                            page: OverlayData::FlashingSuccess(flashing_state.into()),
                            ..inner
                        })
                    }
                    _ => panic!("Unexpected message"),
                },
                _ => panic!("Unexpected message"),
            };

            return show_notification(msg.to_string());
        }
        GemImagerMessage::EditorEvent(evt) => match evt {
            iced::widget::text_editor::Action::Edit(_) => {}
            _ => match state {
                GemImager::FlashingFail(x) => x.logs.perform(evt),
                GemImager::AppInfo(x) => x.license.perform(evt),
                _ => panic!("Unexpected message"),
            },
        },
        GemImagerMessage::AppInfo => {
            *state = GemImager::AppInfo(crate::state::OverlayState::new(
                std::mem::take(state).try_into().expect("Unexpected page"),
            ));

            return state.scroll_reset();
        }
        GemImagerMessage::CopyToClipboard(data) => {
            return iced::clipboard::write(data);
        }
        GemImagerMessage::DbInitSuccess => {
            let db = state.common().db.clone();
            let downloader = state.common().downloader.clone();
            let config_fetch_task =
                Task::future(blocking_future(move || db.remote_configs().unwrap())).then(
                    move |configs| {
                        let dc = downloader.clone();
                        let tasks = configs.into_iter().map(move |(i, u)| {
                            let dc = dc.clone();
                            Task::perform(
                                async move {
                                    let res = helpers::fetch_remote_config(&dc, u).await?;
                                    Ok((i, res))
                                },
                                |x: std::io::Result<(i64, gem_config::config::Config)>| match x {
                                    Ok(y) => GemImagerMessage::ExtendConfig(y),
                                    Err(e) => {
                                        tracing::error!("Failed to fetch config: {e}");
                                        GemImagerMessage::Null
                                    }
                                },
                            )
                        });
                        iced::Task::batch(tasks)
                    },
                );

            let board_icon_task = state.common().fetch_board_images();
            let board_refresh_task = if let GemImager::ChooseBoard(x) = state {
                x.refresh_board_list()
            } else {
                Task::none()
            };

            return Task::batch([board_icon_task, config_fetch_task, board_refresh_task]);
        }
        GemImagerMessage::UpdateSearchText(x) => match state {
            GemImager::ChooseBoard(inner) => return inner.update_search(x),
            GemImager::ChooseOs(inner) => return inner.update_search(x),
            GemImager::ChooseDest(inner) => inner.update_search(x),
            _ => {}
        },
        GemImagerMessage::UpdateInitFormat(f) => {
            if let GemImager::ChooseOs(inner) = state
                && let Some((_, img)) = &mut inner.selected_image
            {
                img.update_init_format(f);
            }
        }
        GemImagerMessage::Null => {}
    }

    Task::none()
}

fn localized_flash_error(lang: gem_i18n::Lang, technical: &str) -> String {
    let lower = technical.to_ascii_lowercase();

    if lower.contains("staging") {
        return format!(
            "{}\n\n{}",
            lang.text(gem_i18n::Msg::StagingSpaceTitle),
            lang.text(gem_i18n::Msg::StagingSpaceBody)
        );
    }

    let ambiguous_dfu =
        lower.contains("devices match") && lower.contains("choose one physical port");
    if ambiguous_dfu
        || lower.contains("dfu")
        || lower.contains("usb")
        || lower.contains("alt-setting")
        || lower.contains("boot manifest")
        || lower.contains("boot artifact")
        || lower.contains("rawemmc")
        || lower.contains("raw emmc")
    {
        let dfu_pair = if ambiguous_dfu {
            Some((
                gem_i18n::Msg::DfuAmbiguousTitle,
                gem_i18n::Msg::DfuAmbiguousBody,
            ))
        } else if lower.contains("no dfu device") {
            Some((
                gem_i18n::Msg::DfuNoDeviceTitle,
                gem_i18n::Msg::DfuNoDeviceBody,
            ))
        } else if lower.contains("boot manifest") || lower.contains("boot artifact") {
            Some((
                gem_i18n::Msg::DfuManifestFailedTitle,
                gem_i18n::Msg::DfuManifestFailedBody,
            ))
        } else if lower.contains("final zero-length packet for `rawemmc`")
            || lower.contains("raw emmc manifest")
            || lower.contains("disconnected before dfuidle")
            || lower.contains("final dfu detach")
        {
            Some((
                gem_i18n::Msg::DfuFinalizeFailedTitle,
                gem_i18n::Msg::DfuFinalizeFailedBody,
            ))
        } else if lower.contains("failed to transfer stage `rawemmc`") {
            Some((
                gem_i18n::Msg::DfuTransferFailedTitle,
                gem_i18n::Msg::DfuTransferFailedBody,
            ))
        } else if lower.contains("driver")
            || lower.contains("winusb")
            || lower.contains("not supported")
            || lower.contains("entity not found")
        {
            Some((
                gem_i18n::Msg::WinusbDriverMissingTitle,
                gem_i18n::Msg::WinusbDriverMissingBody,
            ))
        } else if lower.contains("permission denied")
            || lower.contains("access denied")
            || lower.contains("access is denied")
        {
            Some((
                gem_i18n::Msg::DfuPermissionTitle,
                gem_i18n::Msg::DfuPermissionBody,
            ))
        } else if lower.contains("timed out") || lower.contains("disconnected before") {
            Some((
                gem_i18n::Msg::DfuReconnectTimeoutTitle,
                gem_i18n::Msg::DfuReconnectTimeoutBody,
            ))
        } else if lower.contains("alt-setting") {
            Some((
                gem_i18n::Msg::DfuSwitchToBootModeTitle,
                gem_i18n::Msg::DfuSwitchToBootModeBody,
            ))
        } else {
            None
        };

        if let Some(pair) = dfu_pair {
            return format!("{}\n\n{}", lang.text(pair.0), lang.text(pair.1));
        }
    }

    let pair = if lower.contains("checksum")
        || lower.contains("digest")
        || lower.contains("hash mismatch")
    {
        (
            gem_i18n::Msg::IntegrityFailedTitle,
            gem_i18n::Msg::IntegrityFailedBody,
        )
    } else if lower.contains("read back") || lower.contains("verification failed") {
        (
            gem_i18n::Msg::ReadBackFailedTitle,
            gem_i18n::Msg::ReadBackFailedBody,
        )
    } else if lower.contains("permission denied") || lower.contains("access is denied") {
        (
            gem_i18n::Msg::UdevPermissionTitle,
            gem_i18n::Msg::UdevPermissionBody,
        )
    } else if lower.contains("system disk") || lower.contains("system drive") {
        (
            gem_i18n::Msg::SystemDiskRefusedTitle,
            gem_i18n::Msg::SystemDiskRefusedBody,
        )
    } else if lower.contains("too small")
        || lower.contains("insufficient capacity")
        || lower.contains("not enough space")
    {
        (
            gem_i18n::Msg::DestinationTooSmallTitle,
            gem_i18n::Msg::DestinationTooSmallBody,
        )
    } else if lower.contains("not a recognised removable device") {
        (
            gem_i18n::Msg::UnknownDestinationTitle,
            gem_i18n::Msg::UnknownDestinationBody,
        )
    } else if lower.contains("disconnected")
        || lower.contains("device removed")
        || lower.contains("no such device")
    {
        (
            gem_i18n::Msg::DestinationRemovedTitle,
            gem_i18n::Msg::DestinationRemovedBody,
        )
    } else {
        (
            gem_i18n::Msg::FlashFailedNotification,
            gem_i18n::Msg::GenericFlashFailedBody,
        )
    };

    format!("{}\n\n{}", lang.text(pair.0), lang.text(pair.1))
}

fn show_notification(msg: String) -> Task<GemImagerMessage> {
    Task::future(async move {
        let res = helpers::show_notification(msg).await;
        tracing::debug!("Notification response {res:?}");
        GemImagerMessage::Null
    })
}

#[cfg(test)]
mod i18n_tests {
    use super::localized_flash_error;
    use gem_i18n::Lang;

    #[test]
    fn integrity_failure_is_actionable_in_both_languages() {
        let technical = "Unknown Error during IO: archive checksum mismatch: deadbeef";
        let en = localized_flash_error(Lang::En, technical);
        let tr = localized_flash_error(Lang::Tr, technical);

        assert!(en.contains("checksum"));
        assert!(en.contains("download it again"));
        assert!(tr.contains("sağlama"));
        assert!(tr.contains("yeniden indirin"));
        assert!(!en.contains("deadbeef"));
        assert!(!tr.contains("deadbeef"));
    }

    #[test]
    fn dfu_failures_are_told_apart_and_are_actionable() {
        let driver = localized_flash_error(
            Lang::En,
            "DFU transport error: the WinUSB driver is not bound to this device",
        );
        let permission = localized_flash_error(
            Lang::En,
            "DFU transport error: Permission denied (os error 13)",
        );
        let missing = localized_flash_error(Lang::En, "no DFU device was found at bus 3 port 2.7");
        let wrong_mode = localized_flash_error(
            Lang::En,
            "expected alt-setting `bootloader`; available: [\"rawemmc\"]",
        );
        let interrupted = localized_flash_error(
            Lang::En,
            "DFU: timed out waiting for alt-setting `tispl.bin` at bus 3 port 2.7",
        );
        let ambiguous = localized_flash_error(
            Lang::En,
            "2 devices match 0451:6165; choose one physical port",
        );
        let transfer = localized_flash_error(
            Lang::En,
            "failed to transfer stage `rawemmc`: DFU transport error: Access denied +             (insufficient permissions)",
        );
        let finalization =
            localized_flash_error(Lang::En, "timed out while waiting for raw eMMC manifest");

        assert!(driver.contains("Gem Imager"));
        assert!(permission.contains("udev"));
        assert!(missing.contains("Nothing was written"));
        assert!(wrong_mode.contains("boot switch"));
        assert!(interrupted.contains("did not come back"));
        assert!(ambiguous.contains("all but the board"));
        assert!(transfer.contains("partly written"));
        assert!(finalization.contains("All data was sent"));
        assert!(
            localized_flash_error(
                Lang::En,
                "failed to send the final zero-length packet for `rawemmc`: DFU transport error: +                 Access denied (insufficient permissions)"
            )
            .contains("All data was sent")
        );

        let all = [
            &driver,
            &permission,
            &missing,
            &wrong_mode,
            &interrupted,
            &ambiguous,
            &transfer,
            &finalization,
        ];
        for (i, a) in all.iter().enumerate() {
            for b in all.iter().skip(i + 1) {
                assert_ne!(a, b);
            }
        }

        assert!(!permission.contains("card"));
        assert_ne!(
            permission,
            localized_flash_error(Lang::En, "Permission denied opening /dev/sda")
        );

        for (technical, marker) in [
            (
                "DFU transport error: the WinUSB driver is not bound to this device",
                "tek tıklamayla",
            ),
            (
                "2 devices match 0451:6165; choose one physical port",
                "dışındakileri çıkarın",
            ),
            (
                "failed to transfer stage `rawemmc`: DFU transport error: Access denied +                 (insufficient permissions)",
                "kısmen yazılmış",
            ),
            (
                "timed out while waiting for raw eMMC manifest",
                "Tüm veri gönderildi",
            ),
        ] {
            let tr = localized_flash_error(Lang::Tr, technical);
            assert!(tr.contains(marker), "missing `{marker}` in `{tr}`");
        }
    }

    #[test]
    fn a_listed_but_unopenable_board_still_reaches_its_instruction() {
        let permission = localized_flash_error(
            Lang::En,
            "DFU device permission denied: the board is present but cannot be opened",
        );
        let driver = localized_flash_error(
            Lang::En,
            "DFU device driver missing: no WinUSB-compatible driver is bound to the board",
        );

        assert!(permission.contains("udev"));
        assert!(driver.contains("Gem Imager"));
        assert_ne!(permission, driver);
        assert!(
            localized_flash_error(
                Lang::Tr,
                "DFU device driver missing: no WinUSB-compatible driver is bound to the board"
            )
            .contains("tek tıklamayla")
        );
    }

    #[test]
    fn a_full_disk_during_staging_reads_as_a_host_problem() {
        let technical = "not enough free space for the DFU staging image in C:\\cache: \
                         4352000000 bytes required, 120000000 bytes available";

        let en = localized_flash_error(Lang::En, technical);
        let tr = localized_flash_error(Lang::Tr, technical);

        assert!(en.contains("disk space"));
        assert!(en.contains("board was not touched"));
        assert!(tr.contains("disk alanı"));
        assert!(tr.contains("karta dokunulmadı"));
        assert!(!en.contains("4352000000"));
    }

    #[test]
    fn an_unrecognised_destination_asks_the_user_to_reconnect_the_card() {
        let technical = "Refusing to write to \"/dev/sdb\": it is not a recognised removable \
                         device. Reconnect the card and try again.";

        let en = localized_flash_error(Lang::En, technical);
        let tr = localized_flash_error(Lang::Tr, technical);

        assert!(en.contains("Reconnect the card"), "{en}");
        assert!(tr.contains("Kartı yeniden takın"), "{tr}");
        assert!(!en.contains("Logs"), "{en}");
        assert_ne!(
            en,
            localized_flash_error(
                Lang::En,
                "Refusing to write: it is reported as a system disk."
            )
        );
    }

    #[test]
    fn unknown_technical_failure_points_to_logs_in_both_languages() {
        let en = localized_flash_error(Lang::En, "opaque backend error 47");
        let tr = localized_flash_error(Lang::Tr, "opaque backend error 47");

        assert!(en.contains("Logs"));
        assert!(tr.contains("Günlükler"));
        assert!(!en.contains("error 47"));
        assert!(!tr.contains("error 47"));
    }
}
