//! Global GUI Messages

use iced::Task;

use crate::{
    BBImager,
    helpers::{self, blocking_future},
    state::{OverlayData, OverlayState},
};

#[derive(Debug, Clone)]
pub(crate) enum BBImagerMessage {
    /// Messages to ignore
    Null,

    /// Config related options
    ExtendConfig((i64, bb_config::Config)),
    ResolveRemoteSubitemItem {
        item: Vec<bb_config::config::OsListItem>,
        target: i64,
    },

    /// A new version of application is available
    UpdateAvailable(semver::Version),

    /// Select a board by index. Can only be used in Board selection page.
    UpdateBoardList(Vec<crate::db::BoardListItem>),
    SelectBoardById(i64),
    SelectBoard(crate::db::Board),

    /// ChooseOs Page
    UpdateOsList((Vec<helpers::OsImageItem>, Option<i64>)),
    SelectOs(helpers::OsImageId),
    SelectLocalOs(helpers::BoardImage),
    SelectRemoteOs((crate::db::OsImage, bb_config::config::Flasher)),
    GotoOsListParent,
    UpdateInitFormat(bb_config::config::InitFormat),

    /// Choose Destination page
    SelectDest(helpers::Destination),
    SelectFileDest(String),
    DestinationFilter(bool),

    // Customization Page
    UpdateFlashConfig(crate::helpers::FlashingCustomization),
    ResetFlashingConfig,

    // Review Page
    RequestFlash,
    CancelFlashRequest,
    FlashStart,

    /// Change and persist the interface language.
    SetLanguage(bb_i18n::Lang),

    // Flashing Page
    FlashProgress(bb_flasher::DownloadFlashingStatus),
    FlashSuccess,
    FlashCancel,
    FlashFail(String),

    // Reset to start from beginning.
    Restart,
    // Retry flashing
    Retry,

    /// Open URL in browser
    OpenUrl(url::Url),

    /// Next button pressed
    Next,
    /// Back button pressed
    Back,

    /// Add image to cache
    ResolveImage(url::Url, std::path::PathBuf),
    // Download images which have not already been downloaded
    FilterResolveImages(Vec<url::Url>),

    /// Update destinations
    Destinations(Vec<helpers::Destination>),

    /// Read-only Windows PnP state and the one-click WinUSB helper flow.
    #[cfg(feature = "dfu-driver-mvp")]
    DfuDriverProbe(bb_winusb::DriverState),
    #[cfg(feature = "dfu-driver-mvp")]
    DfuDriverInstall,
    #[cfg(feature = "dfu-driver-mvp")]
    DfuDriverInstallFinished(Result<bb_winusb::InstallOutcome, bb_winusb::InstallError>),
    #[cfg(feature = "dfu-driver-mvp")]
    DfuDriverDismiss,
    #[cfg(feature = "dfu-driver-mvp")]
    DfuDriverShowDetails,
    #[cfg(feature = "dfu-driver-mvp")]
    DfuDriverBackToOffer,

    /// Read-only editor
    EditorEvent(iced::widget::text_editor::Action),

    /// Show application information
    AppInfo,

    /// Copy text to clipboard.
    CopyToClipboard(String),

    /// DB Ops
    DbInitSuccess,

    /// Search
    UpdateSearchText(String),
}

pub(crate) fn update(state: &mut BBImager, message: BBImagerMessage) -> Task<BBImagerMessage> {
    match message {
        BBImagerMessage::SetLanguage(lang) => state.common_mut().set_lang(lang),
        BBImagerMessage::RequestFlash => match state {
            BBImager::Review(inner) => inner.erase_confirmation = true,
            _ => panic!("Unexpected message"),
        },
        BBImagerMessage::CancelFlashRequest => match state {
            BBImager::Review(inner) => inner.erase_confirmation = false,
            _ => panic!("Unexpected message"),
        },
        BBImagerMessage::SelectBoardById(id) => {
            let db = state.common().db.clone();
            return Task::perform(
                blocking_future(move || db.board_by_id(id).expect("Incorrect board id")),
                BBImagerMessage::SelectBoard,
            );
        }
        BBImagerMessage::UpdateBoardList(boards) => {
            // Update board list only if still on that page
            match state {
                BBImager::ChooseBoard(x) => {
                    x.boards = boards;
                }
                BBImager::AppInfo(overlay_state) => match &mut overlay_state.page {
                    OverlayData::ChooseBoard(x) => x.boards = boards,
                    _ => panic!("Unexpected message"),
                },
                _ => panic!("Unexpected message"),
            }
        }
        BBImagerMessage::SelectBoard(b) => match state {
            BBImager::ChooseBoard(inner) => {
                inner.selected_board = Some(b);
            }
            BBImager::AppInfo(overlay_state) => match &mut overlay_state.page {
                OverlayData::ChooseBoard(inner) => inner.selected_board = Some(b),
                _ => panic!("Unexpected message"),
            },
            _ => panic!("Unexpected message"),
        },
        BBImagerMessage::UpdateOsList((imgs, pos)) => {
            match state {
                BBImager::ChooseOs(inner) => inner.update_images(imgs, pos),
                BBImager::AppInfo(overlay_state) => {
                    if let OverlayData::ChooseOs(inner) = &mut overlay_state.page {
                        inner.update_images(imgs, pos)
                    }
                }
                _ => {}
            };
        }
        BBImagerMessage::SelectOs(id) => match state {
            BBImager::ChooseOs(inner) => match id {
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
                            Some(y) => BBImagerMessage::SelectLocalOs(helpers::BoardImage::local(
                                y, flasher,
                            )),
                            None => BBImagerMessage::Null,
                        },
                    );
                }
                helpers::OsImageId::OsImage(id) => {
                    let db = inner.common.db.clone();
                    let flasher = inner.flasher;
                    return Task::perform(
                        blocking_future(move || db.os_image_by_id(id)),
                        move |x| match x {
                            Ok(i) => BBImagerMessage::SelectRemoteOs((i, flasher)),
                            Err(e) => {
                                tracing::error!("Failed to get os image {e}");
                                BBImagerMessage::Null
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
        BBImagerMessage::SelectRemoteOs((image, flasher)) => match state {
            BBImager::ChooseOs(inner) => {
                inner.selected_image = Some((
                    helpers::OsImageId::OsImage(image.id),
                    helpers::BoardImage::remote(image, flasher, inner.common.downloader.clone()),
                ));
            }
            BBImager::AppInfo(overlay_state) => match &mut overlay_state.page {
                OverlayData::ChooseOs(inner) => {
                    inner.selected_image = Some((
                        helpers::OsImageId::OsImage(image.id),
                        helpers::BoardImage::remote(
                            image,
                            flasher,
                            inner.common.downloader.clone(),
                        ),
                    ));
                }
                _ => panic!("Unexpected message"),
            },
            _ => panic!("Unexpected message"),
        },
        BBImagerMessage::SelectLocalOs(image) => match state {
            BBImager::ChooseOs(inner) => {
                inner.selected_image = Some((helpers::OsImageId::Local(image.flasher()), image))
            }
            _ => panic!("Unexpected message"),
        },
        BBImagerMessage::OpenUrl(x) => {
            return Task::future(async move {
                let res = webbrowser::open(x.as_str());
                tracing::debug!("Open Url Resp {res:?}");
                BBImagerMessage::Null
            });
        }
        BBImagerMessage::Next => return state.next(),
        BBImagerMessage::Back => return state.back(),
        BBImagerMessage::ResolveImage(k, v) => state.image_cache_insert(k, v),
        BBImagerMessage::FilterResolveImages(x) => {
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
        BBImagerMessage::ExtendConfig((u, c)) => {
            tracing::debug!("Update Config: {:#?}", c);

            let db = state.common().db.clone();
            let db_task = Task::perform(blocking_future(move || db.add_config(c, Some(u))), |x| {
                if let Err(e) = x {
                    tracing::error!("Failed to merge config {e}");
                }
                BBImagerMessage::Null
            });

            let tail_tasks = match state {
                // If we are in ChooseBoard page, update the board list
                BBImager::ChooseBoard(inner) => Task::batch([
                    inner.common.fetch_board_images(),
                    inner.refresh_board_list(),
                ]),
                BBImager::ChooseOs(inner) => {
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

            // We want fetch board images to run after the config has been added
            return db_task.chain(tail_tasks);
        }
        BBImagerMessage::ResolveRemoteSubitemItem { item, target } => {
            let db = state.common().db.clone();
            let tail = match &state {
                BBImager::ChooseOs(inner) => Task::batch([
                    // Fetch all children remote subitems.
                    inner.resolve_remote_sublists(inner.selected_board.id, Some(target)),
                    inner.refresh_image_list(),
                    state.refresh_image_icons(inner.selected_board.id),
                ]),
                _ => Task::none(),
            };

            return Task::future(blocking_future(move || {
                db.os_remote_sublist_resolve(target, &item).unwrap();
                BBImagerMessage::Null
            }))
            .chain(tail);
        }
        BBImagerMessage::UpdateAvailable(x) => {
            return show_notification(bb_i18n::fmt::update_available(
                state.common().lang(),
                &x.to_string(),
            ));
        }
        BBImagerMessage::GotoOsListParent => match state {
            BBImager::ChooseOs(inner) => {
                let db = inner.common.db.clone();
                let curpos = inner.pos.unwrap();
                let board_id = inner.selected_board.id;
                return Task::perform(
                    blocking_future(move || {
                        let id = db.os_sublist_parent(curpos).unwrap();
                        let imgs = db.os_image_items(board_id, id).unwrap();
                        (imgs, id)
                    }),
                    BBImagerMessage::UpdateOsList,
                );
            }
            _ => panic!("Unexpected message"),
        },
        BBImagerMessage::Destinations(x) => {
            if let BBImager::ChooseDest(inner) = state
                && x != inner.destinations
            {
                // A card that was pulled, or a board that left DFU mode, must not stay selected
                // behind an enabled NEXT button.
                inner.selected_dest =
                    helpers::keep_selected_destination(inner.selected_dest.take(), &x);
                inner.destinations = x;
            }
        }
        #[cfg(feature = "dfu-driver-mvp")]
        BBImagerMessage::DfuDriverProbe(driver_state) => {
            state.common_mut().dfu_driver.on_probe(driver_state);
        }
        #[cfg(feature = "dfu-driver-mvp")]
        BBImagerMessage::DfuDriverInstall => {
            if state.common_mut().dfu_driver.begin_install() {
                return crate::driver_ui::install_task();
            }
        }
        #[cfg(feature = "dfu-driver-mvp")]
        BBImagerMessage::DfuDriverInstallFinished(result) => {
            state.common_mut().dfu_driver.finish_install(result);
        }
        #[cfg(feature = "dfu-driver-mvp")]
        BBImagerMessage::DfuDriverDismiss => {
            state.common_mut().dfu_driver.dismiss();
        }
        #[cfg(feature = "dfu-driver-mvp")]
        BBImagerMessage::DfuDriverShowDetails => {
            state.common_mut().dfu_driver.show_details();
        }
        #[cfg(feature = "dfu-driver-mvp")]
        BBImagerMessage::DfuDriverBackToOffer => {
            state.common_mut().dfu_driver.back_to_offer();
        }
        BBImagerMessage::SelectDest(x) => match state {
            BBImager::ChooseDest(inner) => {
                inner.selected_dest = Some(x);
            }
            _ => panic!("Unexpected message"),
        },
        BBImagerMessage::SelectFileDest(x) => {
            return Task::perform(
                async move {
                    rfd::AsyncFileDialog::new()
                        .set_file_name(x)
                        .save_file()
                        .await
                        .map(|x| x.inner().to_path_buf())
                },
                move |x| match x {
                    Some(y) => BBImagerMessage::SelectDest(helpers::Destination::LocalFile(y)),
                    None => BBImagerMessage::Null,
                },
            );
        }
        BBImagerMessage::DestinationFilter(x) => match state {
            BBImager::ChooseDest(inner) => {
                inner.filter_destination = x;
            }
            _ => panic!("Unexpected message"),
        },
        BBImagerMessage::UpdateFlashConfig(x) => match state {
            BBImager::Customize(inner) => {
                inner.customization = x;
            }
            _ => panic!("Unexpected message"),
        },
        BBImagerMessage::ResetFlashingConfig => match state {
            BBImager::Customize(inner) => {
                inner.customization.reset();
            }
            _ => panic!("Unexpected message"),
        },
        BBImagerMessage::FlashCancel => {
            let lang = state.common().lang();
            let mut msg = lang.text(bb_i18n::Msg::FlashCancelledNotification);

            *state = match std::mem::take(state) {
                BBImager::Flashing(inner) => {
                    inner.cancel_flashing.abort();

                    if inner.is_download {
                        msg = lang.text(bb_i18n::Msg::DownloadCancelledNotification);
                    }
                    BBImager::FlashingCancel(inner.into())
                }
                BBImager::AppInfo(inner) => match inner.page {
                    OverlayData::Flashing(flashing_state) => {
                        flashing_state.cancel_flashing.abort();

                        if flashing_state.is_download {
                            msg = lang.text(bb_i18n::Msg::DownloadCancelledNotification);
                        }

                        BBImager::AppInfo(OverlayState {
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
        BBImagerMessage::Restart => {
            return state.restart();
        }
        BBImagerMessage::FlashFail(err) => {
            let lang = state.common().lang();
            let user_err = localized_flash_error(lang, &err);
            let mut msg = lang.text(bb_i18n::Msg::FlashFailedNotification);

            let logs =
                std::fs::read_to_string(helpers::log_file_path()).expect("Failed to read logs");
            let logs = iced::widget::text_editor::Content::with_text(&logs);

            *state = match std::mem::take(state) {
                BBImager::Flashing(inner) => {
                    if inner.is_download {
                        msg = lang.text(bb_i18n::Msg::DownloadFailedNotification);
                    }

                    BBImager::FlashingFail(crate::state::FlashingFailState {
                        common: inner.common,
                        err: user_err.clone(),
                        logs,
                        selected_board: inner.selected_board,
                        selected_image: inner.selected_image,
                        selected_dest: inner.selected_dest,
                        customization: inner.customization,
                    })
                }
                BBImager::AppInfo(inner) => match inner.page {
                    OverlayData::Flashing(flashing_state) => {
                        if flashing_state.is_download {
                            msg = lang.text(bb_i18n::Msg::DownloadFailedNotification);
                        }

                        BBImager::AppInfo(OverlayState {
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
        BBImagerMessage::FlashProgress(x) => match state {
            BBImager::Flashing(inner) => {
                inner.progress_update(x);
            }
            BBImager::AppInfo(inner) => match &mut inner.page {
                OverlayData::Flashing(flashing_state) => flashing_state.progress_update(x),
                _ => panic!("Unexpected message"),
            },
            // Debug build can be slow.
            _ => {}
        },
        BBImagerMessage::FlashStart | BBImagerMessage::Retry => {
            return state.start_flashing();
        }
        BBImagerMessage::FlashSuccess => {
            let lang = state.common().lang();
            let mut msg = lang.text(bb_i18n::Msg::FlashFinishedNotification);

            *state = match std::mem::take(state) {
                BBImager::Flashing(inner) => {
                    if inner.is_download {
                        msg = lang.text(bb_i18n::Msg::DownloadFinishedNotification);
                    }
                    BBImager::FlashingSuccess(inner.into())
                }
                BBImager::AppInfo(inner) => match inner.page {
                    OverlayData::Flashing(flashing_state) => {
                        if flashing_state.is_download {
                            msg = lang.text(bb_i18n::Msg::DownloadFinishedNotification);
                        }

                        BBImager::AppInfo(OverlayState {
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
        BBImagerMessage::EditorEvent(evt) => match evt {
            iced::widget::text_editor::Action::Edit(_) => {}
            _ => match state {
                BBImager::FlashingFail(x) => x.logs.perform(evt),
                BBImager::AppInfo(x) => x.license.perform(evt),
                _ => panic!("Unexpected message"),
            },
        },
        BBImagerMessage::AppInfo => {
            *state = BBImager::AppInfo(crate::state::OverlayState::new(
                std::mem::take(state).try_into().expect("Unexpected page"),
            ));

            return state.scroll_reset();
        }
        BBImagerMessage::CopyToClipboard(data) => {
            return iced::clipboard::write(data);
        }
        BBImagerMessage::DbInitSuccess => {
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
                                |x: std::io::Result<(i64, bb_config::config::Config)>| match x {
                                    Ok(y) => BBImagerMessage::ExtendConfig(y),
                                    Err(e) => {
                                        tracing::error!("Failed to fetch config: {e}");
                                        BBImagerMessage::Null
                                    }
                                },
                            )
                        });
                        iced::Task::batch(tasks)
                    },
                );

            let board_icon_task = state.common().fetch_board_images();
            let board_refresh_task = if let BBImager::ChooseBoard(x) = state {
                x.refresh_board_list()
            } else {
                Task::none()
            };

            return Task::batch([board_icon_task, config_fetch_task, board_refresh_task]);
        }
        BBImagerMessage::UpdateSearchText(x) => match state {
            BBImager::ChooseBoard(inner) => return inner.update_search(x),
            BBImager::ChooseOs(inner) => return inner.update_search(x),
            BBImager::ChooseDest(inner) => inner.update_search(x),
            _ => {}
        },
        BBImagerMessage::UpdateInitFormat(f) => {
            if let BBImager::ChooseOs(inner) = state
                && let Some((_, img)) = &mut inner.selected_image
            {
                img.update_init_format(f);
            }
        }
        BBImagerMessage::Null => {}
    }

    Task::none()
}

/// Turn low-level flasher chains into an actionable, translated message.
///
/// The complete chain is written to the log before this message reaches the reducer. The finish
/// screen intentionally receives only this safe summary, so implementation details do not become
/// the sole explanation offered to the user.
fn localized_flash_error(lang: bb_i18n::Lang, technical: &str) -> String {
    let lower = technical.to_ascii_lowercase();

    // The staging image is written before the board is touched, so its failure has to read as a
    // host-disk problem rather than as a flashing problem.
    if lower.contains("staging") {
        return format!(
            "{}\n\n{}",
            lang.text(bb_i18n::Msg::StagingSpaceTitle),
            lang.text(bb_i18n::Msg::StagingSpaceBody)
        );
    }

    // DFU failures are matched before the generic ones. "Permission denied" on a DFU device and on
    // an SD card need different answers — a udev rule for the USB device versus for the block
    // device — and the Windows driver case has no SD equivalent at all. Falling through to the
    // shared wording would send the user to fix the wrong thing.
    // Not every DFU failure names DFU: the alt-setting and boot-manifest errors are phrased in the
    // protocol's own terms, and they are the two that most need their own answer.
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
            // Two boards on one host: refusing to guess is the safe behaviour, and the message has
            // to say that nothing was written.
            Some((
                bb_i18n::Msg::DfuAmbiguousTitle,
                bb_i18n::Msg::DfuAmbiguousBody,
            ))
        } else if lower.contains("no dfu device") {
            Some((
                bb_i18n::Msg::DfuNoDeviceTitle,
                bb_i18n::Msg::DfuNoDeviceBody,
            ))
        } else if lower.contains("boot manifest") || lower.contains("boot artifact") {
            Some((
                bb_i18n::Msg::DfuManifestFailedTitle,
                bb_i18n::Msg::DfuManifestFailedBody,
            ))
        } else if lower.contains("final zero-length packet for `rawemmc`")
            || lower.contains("raw emmc manifest")
            || lower.contains("disconnected before dfuidle")
            || lower.contains("final dfu detach")
        {
            // All raw bytes have been handed to the board. Failure from the ZLP onwards is a
            // finalization failure, not a reconnect timeout or a mid-stream transfer failure.
            Some((
                bb_i18n::Msg::DfuFinalizeFailedTitle,
                bb_i18n::Msg::DfuFinalizeFailedBody,
            ))
        } else if lower.contains("failed to transfer stage `rawemmc`") {
            Some((
                bb_i18n::Msg::DfuTransferFailedTitle,
                bb_i18n::Msg::DfuTransferFailedBody,
            ))
        } else if lower.contains("driver")
            || lower.contains("winusb")
            || lower.contains("not supported")
            || lower.contains("entity not found")
        {
            Some((
                bb_i18n::Msg::WinusbDriverMissingTitle,
                bb_i18n::Msg::WinusbDriverMissingBody,
            ))
        } else if lower.contains("permission denied")
            || lower.contains("access denied")
            || lower.contains("access is denied")
        {
            Some((
                bb_i18n::Msg::DfuPermissionTitle,
                bb_i18n::Msg::DfuPermissionBody,
            ))
        } else if lower.contains("timed out") || lower.contains("disconnected before") {
            // Checked before the alt-setting branch: the reconnect timeout also names an
            // alt-setting ("timed out waiting for alt-setting `tispl.bin`"), but the board coming
            // back late is a different problem from the board being in the wrong mode.
            Some((
                bb_i18n::Msg::DfuReconnectTimeoutTitle,
                bb_i18n::Msg::DfuReconnectTimeoutBody,
            ))
        } else if lower.contains("alt-setting") {
            // The board is attached but not in the mode this stage needs, so the switch position
            // is the thing to check — not the cable, and not the driver.
            Some((
                bb_i18n::Msg::DfuSwitchToBootModeTitle,
                bb_i18n::Msg::DfuSwitchToBootModeBody,
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
            bb_i18n::Msg::IntegrityFailedTitle,
            bb_i18n::Msg::IntegrityFailedBody,
        )
    } else if lower.contains("read back") || lower.contains("verification failed") {
        (
            bb_i18n::Msg::ReadBackFailedTitle,
            bb_i18n::Msg::ReadBackFailedBody,
        )
    } else if lower.contains("permission denied") || lower.contains("access is denied") {
        (
            bb_i18n::Msg::UdevPermissionTitle,
            bb_i18n::Msg::UdevPermissionBody,
        )
    } else if lower.contains("system disk") || lower.contains("system drive") {
        (
            bb_i18n::Msg::SystemDiskRefusedTitle,
            bb_i18n::Msg::SystemDiskRefusedBody,
        )
    } else if lower.contains("too small")
        || lower.contains("insufficient capacity")
        || lower.contains("not enough space")
    {
        (
            bb_i18n::Msg::DestinationTooSmallTitle,
            bb_i18n::Msg::DestinationTooSmallBody,
        )
    } else if lower.contains("disconnected")
        || lower.contains("device removed")
        || lower.contains("no such device")
    {
        (
            bb_i18n::Msg::DestinationRemovedTitle,
            bb_i18n::Msg::DestinationRemovedBody,
        )
    } else {
        (
            bb_i18n::Msg::FlashFailedNotification,
            bb_i18n::Msg::GenericFlashFailedBody,
        )
    };

    format!("{}\n\n{}", lang.text(pair.0), lang.text(pair.1))
}

fn show_notification(msg: String) -> Task<BBImagerMessage> {
    Task::future(async move {
        let res = helpers::show_notification(msg).await;
        tracing::debug!("Notification response {res:?}");
        BBImagerMessage::Null
    })
}

#[cfg(test)]
mod i18n_tests {
    use super::localized_flash_error;
    use bb_i18n::Lang;

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

    /// The DFU failures a user can actually act on must each say something different, in both
    /// languages. Collapsing them into the shared "flashing failed" wording — or into the SD card's
    /// udev advice — sends the user to fix the wrong thing.
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

        // Distinct remedies, not one generic wording reused.
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

        // A DFU permission failure must not be answered with the SD card's disk advice.
        assert!(!permission.contains("card"));
        assert_ne!(
            permission,
            localized_flash_error(Lang::En, "Permission denied opening /dev/sda")
        );

        // Turkish carries the same distinctions rather than falling back to English.
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

    /// A board that is listed but cannot be opened is refused before any download, and that
    /// refusal has to arrive as the same actionable instruction the deeper backend errors produce.
    /// These are the exact sentences `helpers::flash` emits, so the coupling is asserted rather
    /// than assumed.
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

    /// The staging image is prepared on the host before the board is touched, so running out of
    /// disk has to read as a host problem — and has to say the board was left alone.
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
        // The raw byte counts belong in the log, not on the finish screen.
        assert!(!en.contains("4352000000"));
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
