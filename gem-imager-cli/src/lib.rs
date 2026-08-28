pub mod cli;

use anyhow::Context as _;
use clap::CommandFactory;
use cli::{Commands, DestinationsTarget, Opt, TargetCommands};
use gem_downloader::{ArchiveIntegrity, Downloader};
use gem_flasher::{DownloadFlashingStatus, GemFlasherTarget, LocalImage};
use std::path::PathBuf;
use std::sync::mpsc;

pub fn run(opt: Opt) -> anyhow::Result<()> {
    match opt.command {
        Commands::Flash { target, quiet } => return flash(*target, quiet),
        Commands::Format { dst, quiet } => return format(dst, quiet),
        Commands::ListDestinations {
            target,
            no_frills,
            no_filter,
        } => list_destinations(target, no_frills, no_filter),
        Commands::GenerateCompletion { shell } => generate_completion(shell),
    }

    Ok(())
}

fn flash(target: TargetCommands, quite: bool) -> anyhow::Result<()> {
    if quite {
        flash_internal(target, None)
    } else {
        std::thread::scope(|s| {
            let (tx, rx) = mpsc::sync_channel(2);

            s.spawn(move || {
                let term = console::Term::stdout();
                let bar_style = indicatif::ProgressStyle::with_template(
                    "{msg:15}  [{wide_bar}] [{percent:3} %]",
                )
                .expect("Failed to create progress bar");
                let bars = indicatif::MultiProgress::new();

                let mut last_bar: Option<indicatif::ProgressBar> = None;
                let mut last_state = DownloadFlashingStatus::Preparing;
                let mut stage = 1;

                term.write_line(&stage_msg(DownloadFlashingStatus::Preparing, stage))
                    .unwrap();

                while let Ok(progress) = rx.recv() {
                    if progress == last_state {
                        continue;
                    }

                    match (progress, last_state) {
                        (
                            DownloadFlashingStatus::DownloadingProgress(p),
                            DownloadFlashingStatus::DownloadingProgress(_),
                        )
                        | (
                            DownloadFlashingStatus::FlashingProgress(p),
                            DownloadFlashingStatus::FlashingProgress(_),
                        )
                        | (
                            DownloadFlashingStatus::Verifying(p),
                            DownloadFlashingStatus::Verifying(_),
                        ) => {
                            last_bar.as_ref().unwrap().set_position((p * 100.0) as u64);
                        }
                        (
                            DownloadFlashingStatus::RawWrite(p),
                            DownloadFlashingStatus::RawWrite(_),
                        )
                        | (
                            DownloadFlashingStatus::BootStage { progress: p, .. },
                            DownloadFlashingStatus::BootStage { .. },
                        )
                        | (
                            DownloadFlashingStatus::ChecksummingImage(p),
                            DownloadFlashingStatus::ChecksummingImage(_),
                        ) => {
                            last_bar.as_ref().unwrap().set_position((p * 100.0) as u64);
                        }
                        (DownloadFlashingStatus::DownloadingProgress(p), _)
                        | (DownloadFlashingStatus::FlashingProgress(p), _)
                        | (DownloadFlashingStatus::RawWrite(p), _)
                        | (DownloadFlashingStatus::BootStage { progress: p, .. }, _)
                        | (DownloadFlashingStatus::ChecksummingImage(p), _)
                        | (DownloadFlashingStatus::Verifying(p), _) => {
                            if let Some(b) = last_bar.take() {
                                b.finish();
                            }

                            stage += 1;

                            let temp_bar = bars.add(indicatif::ProgressBar::new(100));
                            temp_bar.set_style(bar_style.clone());
                            temp_bar.set_message(stage_msg(progress, stage));
                            temp_bar.set_position((p * 100.0) as u64);
                            last_bar = Some(temp_bar);
                        }
                        (DownloadFlashingStatus::Customizing, _)
                        | (DownloadFlashingStatus::ResolvingBootArtifacts, _)
                        | (DownloadFlashingStatus::Reconnecting, _)
                        | (DownloadFlashingStatus::Finalizing, _)
                        | (DownloadFlashingStatus::Preparing, _) => {
                            if let Some(b) = last_bar.take() {
                                b.finish();
                            }

                            stage += 1;
                            term.write_line(&stage_msg(progress, stage)).unwrap();
                        }
                    }

                    last_state = progress;
                }

                if let Some(b) = last_bar.take() {
                    b.finish();
                }
            });

            flash_internal(target, Some(tx))
        })
    }
}

fn flash_internal(
    target: TargetCommands,
    chan: Option<mpsc::SyncSender<DownloadFlashingStatus>>,
) -> anyhow::Result<()> {
    match target {
        TargetCommands::Sd {
            dst,
            image_sha256,
            hostname,
            timezone,
            keymap,
            user_name,
            user_password,
            wifi_ssid,
            wifi_password,
            img,
            ssh_key,
            usb_enable_dhcp,
            sysconfig,
            cloud_init,
            file_destination,
        } => {
            // TODO: Remove fallback in the future.
            if !sysconfig && !cloud_init {
                tracing::warn!("No config format specified. Using sysconfig by default");
            }

            let user = user_name.map(|x| (x, user_password.unwrap()));
            let wifi = wifi_ssid.map(|x| (x, wifi_password.unwrap()));

            let dst = check_macos_device_path(dst);

            let target = if file_destination {
                None
            } else {
                Some(
                    dst.clone()
                        .try_into()
                        .map_err(|err| anyhow::anyhow!("{}: {err}", dst.display()))?,
                )
            };

            let img_path = resolve_image(&img, image_sha256.as_deref(), chan.is_some())?;
            tracing::info!("Resolved image: {}", img_path.display());

            let customization = if hostname.is_some()
                || timezone.is_some()
                || keymap.is_some()
                || user.is_some()
                || wifi.is_some()
                || ssh_key.is_some()
                || usb_enable_dhcp
            {
                let mut customization = gem_flasher::sd::FlashingSdLinuxConfig::sysconfig(
                    hostname.clone(),
                    timezone.clone(),
                    keymap.clone(),
                    user.clone(),
                    wifi.clone(),
                    ssh_key.clone(),
                    Some(usb_enable_dhcp),
                );

                if cloud_init {
                    customization.extend([gem_flasher::sd::FlashingSdLinuxConfig::cloud_init(
                        hostname, timezone, keymap, user, wifi, ssh_key,
                    )]);
                }

                customization
            } else {
                gem_flasher::sd::FlashingSdLinuxConfig::none()
            };

            tracing::info!("Customization: {:#?}", customization);

            match target {
                None => gem_flasher::sd::Flasher::with_file_dest(
                    LocalImage::new(img_path.clone().into_boxed_path()).into_image_fn(),
                    dst,
                    customization,
                ),
                Some(target) => gem_flasher::sd::Flasher::new(
                    LocalImage::new(img_path.into_boxed_path()).into_image_fn(),
                    target,
                    customization,
                ),
            }
            .flash(chan, None)
        }
        #[cfg(feature = "dfu")]
        TargetCommands::Dfu {
            identifier,
            image,
            cache_dir,
        } => {
            let flasher = gem_flasher::dfu::Flasher::from_identifier(
                LocalImage::new(image.into()).into_image_fn(),
                &identifier,
                None,
            )
            .map_err(|err| anyhow::anyhow!("DFU device {identifier}: {err}"))?;
            match cache_dir {
                Some(cache_dir) => flasher.with_cache_dir(cache_dir).flash(chan),
                None => flasher.flash(chan),
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn check_macos_device_path(dst: PathBuf) -> PathBuf {
    if dst.to_string_lossy().starts_with("/dev/disk")
        && !dst.to_string_lossy().starts_with("/dev/rdisk")
    {
        let rdisk = dst.to_string_lossy().replace("/dev/disk", "/dev/rdisk");
        if std::path::Path::new(&rdisk).exists() {
            let term = console::Term::stderr();
            let _ = term.write_line(&format!(
                "{} You are using a buffered device path: {}\n\
                 {} For significantly faster flashing, use the raw device path: {}\n",
                console::style("Warning:").yellow().bold(),
                dst.display(),
                console::style("Tip:").green().bold(),
                rdisk
            ));

            let _ = term.write_str(&format!(
                "Do you want to switch to {}? [Y/n] ",
                console::style(&rdisk).bold()
            ));

            let mut input = String::new();
            std::io::stdin()
                .read_line(&mut input)
                .expect("Failed to read line");

            let input = input.trim().to_lowercase();
            if input.is_empty() || input == "y" || input == "yes" {
                let _ = term.write_line(&format!("Switching to {}\n", rdisk));
                return PathBuf::from(rdisk);
            }
        }
    }

    dst
}

#[cfg(not(target_os = "macos"))]
fn check_macos_device_path(dst: PathBuf) -> PathBuf {
    dst
}

fn format(dst: PathBuf, quiet: bool) -> anyhow::Result<()> {
    let term = console::Term::stdout();

    let target = dst
        .clone()
        .try_into()
        .map_err(|err| anyhow::anyhow!("{}: {err}", dst.display()))?;
    gem_flasher::sd::FormatFlasher::new(target)
        .flash()
        .map_err(|err| anyhow::anyhow!("formatting {}: {err}", dst.display()))?;

    if !quiet {
        let _ = term.write_line("Formatting successful");
    }

    Ok(())
}

fn no_frills_list_destinations<T: GemFlasherTarget + Send + 'static>(no_filter: bool) {
    let term = console::Term::stdout();
    let dsts = T::destinations(!no_filter);

    for d in dsts {
        term.write_line(&d.identifier()).unwrap();
    }
}

fn list_destinations(target: DestinationsTarget, no_frills: bool, no_filter: bool) {
    if no_frills {
        match target {
            DestinationsTarget::Sd => {
                no_frills_list_destinations::<gem_flasher::sd::Target>(no_filter)
            }
            #[cfg(feature = "dfu")]
            DestinationsTarget::Dfu => {
                no_frills_list_destinations::<gem_flasher::dfu::Target>(no_filter)
            }
        }
        return;
    }

    let term = console::Term::stdout();

    match target {
        DestinationsTarget::Sd => {
            const NAME_HEADER: &str = "SD Card";
            const PATH_HEADER: &str = "Path";
            const SIZE_HEADER: &str = "Size (in G)";
            const BYTES_IN_GB: u64 = 1024 * 1024 * 1024;

            let dsts_str: Vec<_> = gem_flasher::sd::Target::destinations(!no_filter)
                .into_iter()
                .map(|x| {
                    (
                        x.to_string().trim().to_string(),
                        x.identifier().to_string(),
                        (x.size() / BYTES_IN_GB).to_string(),
                    )
                })
                .collect();

            let max_name_len = dsts_str
                .iter()
                .map(|x| x.0.len())
                .chain([NAME_HEADER.len()])
                .max()
                .unwrap();
            let max_path_len = dsts_str
                .iter()
                .map(|x| x.1.len())
                .chain([PATH_HEADER.len()])
                .max()
                .unwrap();
            let max_size_len = dsts_str
                .iter()
                .map(|x| x.2.len())
                .chain([SIZE_HEADER.len()])
                .max()
                .unwrap();

            let table_border = format!(
                "+-{}-+-{}-+-{}-+",
                std::iter::repeat_n('-', max_name_len).collect::<String>(),
                std::iter::repeat_n('-', max_path_len).collect::<String>(),
                std::iter::repeat_n('-', SIZE_HEADER.len()).collect::<String>(),
            );

            term.write_line(&table_border).unwrap();

            term.write_line(&format!(
                "| {} | {} | {: <6} |",
                console::pad_str(NAME_HEADER, max_name_len, console::Alignment::Left, None),
                console::pad_str(PATH_HEADER, max_path_len, console::Alignment::Left, None),
                console::pad_str(SIZE_HEADER, max_size_len, console::Alignment::Left, None),
            ))
            .unwrap();

            term.write_line(&table_border).unwrap();

            for d in dsts_str {
                term.write_line(&format!(
                    "| {} | {} | {} |",
                    console::pad_str(&d.0, max_name_len, console::Alignment::Left, None),
                    console::pad_str(&d.1, max_path_len, console::Alignment::Left, None),
                    console::pad_str(&d.2, max_size_len, console::Alignment::Right, None)
                ))
                .unwrap();
            }

            term.write_line(&table_border).unwrap();
        }
        #[cfg(feature = "dfu")]
        DestinationsTarget::Dfu => {
            const NAME_HEADER: &str = "Device";
            const BUS_NUMBER_HEADER: &str = "Bus Number";
            const ADDRESS_HEADER: &str = "Address";
            const VENDOR_ID_HEADER: &str = "Vendor Id";
            const PRODUCT_ID_HEADER: &str = "Product Id";

            let dsts_str: Vec<_> = gem_flasher::dfu::Target::destinations(!no_filter)
                .into_iter()
                .map(|x| {
                    (
                        x.to_string().trim().to_string(),
                        format!("{:#04x}", x.bus_number()),
                        format!("{:#04x}", x.port_num()),
                        format!("{:#06x}", x.vendor_id()),
                        format!("{:#06x}", x.product_id()),
                    )
                })
                .collect();

            let max_name_len = dsts_str
                .iter()
                .map(|x| x.0.len())
                .chain([NAME_HEADER.len()])
                .max()
                .unwrap();

            let table_border = format!(
                "+-{}-+-{}-+-{}-+-{}-+-{}-+",
                std::iter::repeat_n('-', max_name_len).collect::<String>(),
                std::iter::repeat_n('-', BUS_NUMBER_HEADER.len()).collect::<String>(),
                std::iter::repeat_n('-', ADDRESS_HEADER.len()).collect::<String>(),
                std::iter::repeat_n('-', VENDOR_ID_HEADER.len()).collect::<String>(),
                std::iter::repeat_n('-', PRODUCT_ID_HEADER.len()).collect::<String>(),
            );

            term.write_line(&table_border).unwrap();

            term.write_line(&format!(
                "| {} | {} | {} | {} | {} |",
                console::pad_str(NAME_HEADER, max_name_len, console::Alignment::Left, None),
                BUS_NUMBER_HEADER,
                ADDRESS_HEADER,
                VENDOR_ID_HEADER,
                PRODUCT_ID_HEADER,
            ))
            .unwrap();

            term.write_line(&table_border).unwrap();

            for d in dsts_str {
                term.write_line(&format!(
                    "| {} | {} | {} | {} | {} |",
                    console::pad_str(&d.0, max_name_len, console::Alignment::Left, None),
                    console::pad_str(
                        &d.1,
                        BUS_NUMBER_HEADER.len(),
                        console::Alignment::Right,
                        None
                    ),
                    console::pad_str(&d.2, ADDRESS_HEADER.len(), console::Alignment::Right, None),
                    console::pad_str(
                        &d.3,
                        VENDOR_ID_HEADER.len(),
                        console::Alignment::Right,
                        None
                    ),
                    console::pad_str(
                        &d.4,
                        PRODUCT_ID_HEADER.len(),
                        console::Alignment::Right,
                        None
                    ),
                ))
                .unwrap();
            }

            term.write_line(&table_border).unwrap();
        }
    }
}

const fn progress_msg(status: DownloadFlashingStatus) -> &'static str {
    match status {
        DownloadFlashingStatus::Preparing => "Preparing  ",
        DownloadFlashingStatus::DownloadingProgress(_) => "Downloading",
        DownloadFlashingStatus::FlashingProgress(_) => "Flashing",
        DownloadFlashingStatus::Verifying(_) => "Verifying",
        DownloadFlashingStatus::Customizing => "Customizing",
        DownloadFlashingStatus::ResolvingBootArtifacts => "Boot files",
        DownloadFlashingStatus::ChecksummingImage(_) => "Checksum",
        DownloadFlashingStatus::Reconnecting => "Reconnecting",
        DownloadFlashingStatus::BootStage { .. } => "Bootloader",
        DownloadFlashingStatus::RawWrite(_) => "eMMC write",
        DownloadFlashingStatus::Finalizing => "Finishing",
    }
}

fn stage_msg(status: DownloadFlashingStatus, stage: usize) -> String {
    format!("[{stage}] {}", progress_msg(status))
}

fn generate_completion(target: clap_complete::Shell) {
    let mut cmd = Opt::command();
    const BIN_NAME: &str = env!("CARGO_PKG_NAME");

    clap_complete::generate(target, &mut cmd, BIN_NAME, &mut std::io::stdout())
}

fn is_remote_source(img: &str) -> bool {
    img.starts_with("https://") || img.starts_with("http://")
}

fn decode_sha256(hex: &str) -> anyhow::Result<[u8; 32]> {
    let hex = hex.trim();
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        anyhow::bail!("--image-sha256 must be 64 hexadecimal characters");
    }

    let mut digest = [0u8; 32];
    for (i, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)?;
    }
    Ok(digest)
}

fn cli_cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("GEM_IMAGER_CACHE_DIR") {
        return PathBuf::from(dir);
    }

    #[cfg(target_os = "windows")]
    return std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("gem-imager-cli");

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        home.join("Library/Caches/gem-imager-cli")
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let cache_home = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .unwrap_or_else(std::env::temp_dir);
        cache_home.join("gem-imager-cli")
    }
}

fn resolve_image(img: &str, sha_hex: Option<&str>, show_progress: bool) -> anyhow::Result<PathBuf> {
    let sha = sha_hex.map(decode_sha256).transpose()?;

    if !is_remote_source(img) {
        let path = PathBuf::from(img);

        if let Some(sha) = sha {
            let actual = gem_downloader::Downloader::sha256_of_file(&path)
                .map_err(|err| anyhow::anyhow!("hashing {} failed: {err}", path.display()))?;
            if actual != sha {
                anyhow::bail!("{} does not match --image-sha256", path.display());
            }
        }

        return Ok(path);
    }

    let Some(sha) = sha else {
        anyhow::bail!(
            "flashing from {img} requires --image-sha256 <hex> so the download can be verified"
        );
    };

    let cache_dir = cli_cache_dir();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building the download runtime")?;

    let bar = show_progress.then(|| {
        let bar = indicatif::ProgressBar::new_spinner();
        bar.set_style(
            indicatif::ProgressStyle::with_template(
                "{msg:15}  {spinner} {bytes} ({bytes_per_sec})",
            )
            .expect("Failed to create download spinner"),
        );
        bar.set_message("Downloading");
        bar.enable_steady_tick(std::time::Duration::from_millis(120));
        bar
    });

    let result = runtime.block_on(async {
        let downloader = Downloader::new(&cache_dir).context("preparing the image cache")?;
        downloader
            .download_archive(img, ArchiveIntegrity::from_sha256(sha), |received| {
                if let Some(bar) = bar.as_ref() {
                    bar.set_position(received);
                }
            })
            .await
            .map_err(|err| anyhow::anyhow!("downloading {img}: {err}"))
    });

    if let Some(bar) = bar {
        bar.finish_and_clear();
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_msg_maps_each_status() {
        assert_eq!(
            progress_msg(DownloadFlashingStatus::Preparing),
            "Preparing  "
        );
        assert_eq!(
            progress_msg(DownloadFlashingStatus::DownloadingProgress(0.5)),
            "Downloading"
        );
        assert_eq!(
            progress_msg(DownloadFlashingStatus::FlashingProgress(0.5)),
            "Flashing"
        );
        assert_eq!(
            progress_msg(DownloadFlashingStatus::Verifying(0.5)),
            "Verifying"
        );
        assert_eq!(
            progress_msg(DownloadFlashingStatus::Customizing),
            "Customizing"
        );
    }

    #[test]
    fn stage_msg_prefixes_stage_number() {
        assert_eq!(
            stage_msg(DownloadFlashingStatus::Preparing, 3),
            "[3] Preparing  "
        );
        assert_eq!(
            stage_msg(DownloadFlashingStatus::Verifying(0.0), 1),
            "[1] Verifying"
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn check_macos_device_path_is_identity_off_macos() {
        let dst = PathBuf::from("/dev/disk2");
        assert_eq!(check_macos_device_path(dst.clone()), dst);
    }

    #[test]
    fn flash_sd_rejects_a_bad_device_before_touching_the_network() {
        let target = crate::cli::TargetCommands::Sd {
            dst: PathBuf::from("/dev/definitely-not-a-real-sd-target"),
            image_sha256: Some("a".repeat(64)),
            hostname: None,
            timezone: None,
            keymap: None,
            user_name: None,
            user_password: None,
            wifi_ssid: None,
            wifi_password: None,
            img: "https://gem-imager-cli.invalid/os.img.xz".to_string(),
            ssh_key: None,
            usb_enable_dhcp: false,
            sysconfig: true,
            cloud_init: false,
            file_destination: false,
        };

        let err = flash_internal(target, None).expect_err("a bad SD device must fail");
        let message = format!("{err:#}");
        assert!(
            message.contains("/dev/definitely-not-a-real-sd-target"),
            "error should name the offending device, got: {message}"
        );
    }
}
