use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(version, about)]
pub struct Opt {
    #[command(subcommand)]
    /// Specifies the subcommand to execute.
    pub command: Commands,
    #[arg(long)]
    /// Enable more logging.
    pub verbose: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Command to flash an image to a specific destination.
    Flash {
        #[command(subcommand)]
        /// Type of BeagleBoard to flash
        target: Box<TargetCommands>,

        #[arg(long)]
        /// Suppress standard output messages for a quieter experience.
        quiet: bool,
    },

    /// Command to list available destinations for flashing based on the selected target.
    ListDestinations {
        /// Specifies the target type for listing destinations.
        target: DestinationsTarget,

        #[arg(long)]
        /// Only print paths seperated by newline
        no_frills: bool,

        #[arg(long)]
        /// Show all possible destinations without any sanity filters. Can be used when a device is
        /// not visible due to incorrect reporting by OS.
        no_filter: bool,
    },

    /// Command to format SD Card
    Format {
        /// The destination device (e.g., `/dev/sdX` or specific device identifiers).
        dst: PathBuf,

        #[arg(long)]
        /// Suppress standard output messages for a quieter experience.
        quiet: bool,
    },

    /// Command to generate shell completion
    GenerateCompletion {
        /// Specifies the target shell type for completion
        shell: clap_complete::Shell,
    },
}

#[derive(Subcommand, Debug)]
pub enum TargetCommands {
    /// Flash an SD card with customizable settings for BeagleBoard devices.
    Sd {
        /// Local path to image file. Can be compressed (xz) or extracted file
        img: Box<Path>,

        /// The destination device (e.g., `/dev/sdX` or specific device identifiers).
        dst: PathBuf,

        #[arg(long)]
        /// Set a custom hostname for the device (e.g., "beaglebone").
        hostname: Option<Box<str>>,

        #[arg(long)]
        /// Set the timezone for the device (e.g., "America/New_York").
        timezone: Option<Box<str>>,

        #[arg(long)]
        /// Set the keyboard layout/keymap (e.g., "us" for the US layout).
        keymap: Option<Box<str>>,

        #[arg(long, requires = "user_password", verbatim_doc_comment)]
        /// Set a username for the default user. Cannot be `root`. Requires `user_password`.
        /// Required to enter GUI session due to regulatory requirements.
        user_name: Option<Box<str>>,

        #[arg(long, requires = "user_name", verbatim_doc_comment)]
        /// Set a password for the default user. Requires `user_name`.
        /// Required to enter GUI session due to regulatory requirements.
        user_password: Option<Box<str>>,

        #[arg(long, requires = "wifi_password")]
        /// Configure a Wi-Fi SSID for network access. Requires `wifi_password`.
        wifi_ssid: Option<Box<str>>,

        #[arg(long, requires = "wifi_ssid")]
        /// Set the password for the specified Wi-Fi SSID. Requires `wifi_ssid`.
        wifi_password: Option<Box<str>>,

        #[arg(long)]
        /// Set SSH public key for authentication
        ssh_key: Option<Box<str>>,

        #[arg(long)]
        /// Enable USB DHCP
        usb_enable_dhcp: bool,

        #[arg(long)]
        /// Generate clound-init config.
        cloud_init: bool,

        #[arg(long)]
        /// Generate sysconfig. Currently, sysconfig will be generated regardless if this flag is
        /// provides. However, this will change in future. So best to explicitly set the flag.
        sysconfig: bool,

        /// The destination is a file instead of SD Card
        #[arg(long)]
        file_destination: bool,
    },
    #[cfg(feature = "dfu")]
    Dfu {
        /// Identifier format: `{bus}:{physical-port-path}:{vendor}:{product}` in hexadecimal.
        /// A hub path is dot-separated, for example `03:02.07:0451:6165`.
        identifier: String,
        /// Extracted/customized raw image to stream to eMMC. Boot artifacts are resolved from the
        /// verified T3 manifest and must not be supplied manually.
        image: PathBuf,
        /// Override the persistent content-addressed DFU cache (primarily for parity testing).
        #[arg(long)]
        cache_dir: Option<PathBuf>,
    },
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum DestinationsTarget {
    /// SD card targets.
    Sd,
    /// USB DFU Target
    #[cfg(feature = "dfu")]
    Dfu,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// clap's own recommended smoke test: validates the entire derived command
    /// tree (no duplicate args, well-formed `requires`/`conflicts`, etc.).
    #[test]
    fn cli_definition_is_valid() {
        Opt::command().debug_assert();
    }

    #[test]
    fn flash_sd_minimal_parses() {
        let opt = Opt::try_parse_from(["bb-imager-cli", "flash", "sd", "img.xz", "/dev/sdX"])
            .expect("valid sd flash invocation");
        assert!(!opt.verbose);
        match opt.command {
            Commands::Flash { target, quiet } => {
                assert!(!quiet);
                match *target {
                    TargetCommands::Sd { img, dst, .. } => {
                        assert_eq!(img.as_ref(), Path::new("img.xz"));
                        assert_eq!(dst, PathBuf::from("/dev/sdX"));
                    }
                    other => panic!("expected Sd, got {other:?}"),
                }
            }
            other => panic!("expected Flash, got {other:?}"),
        }
    }

    #[cfg(feature = "dfu")]
    #[test]
    fn flash_dfu_parses_physical_path_image_and_cache() {
        let opt = Opt::try_parse_from([
            "bb-imager-cli",
            "flash",
            "dfu",
            "03:02.07:0451:6165",
            "staging.img",
            "--cache-dir",
            "dfu-cache",
        ])
        .expect("valid T3 DFU parity invocation");
        match opt.command {
            Commands::Flash { target, .. } => match *target {
                TargetCommands::Dfu {
                    identifier,
                    image,
                    cache_dir,
                } => {
                    assert_eq!(identifier, "03:02.07:0451:6165");
                    assert_eq!(image, PathBuf::from("staging.img"));
                    assert_eq!(cache_dir, Some(PathBuf::from("dfu-cache")));
                }
                other => panic!("expected Dfu, got {other:?}"),
            },
            other => panic!("expected Flash, got {other:?}"),
        }
    }

    #[test]
    fn flash_sd_customization_flags_parse() {
        let opt = Opt::try_parse_from([
            "bb-imager-cli",
            "flash",
            "sd",
            "img.xz",
            "/dev/sdX",
            "--hostname",
            "beagle",
            "--usb-enable-dhcp",
            "--file-destination",
        ])
        .expect("valid customized sd flash");
        match opt.command {
            Commands::Flash { target, .. } => match *target {
                TargetCommands::Sd {
                    hostname,
                    usb_enable_dhcp,
                    file_destination,
                    ..
                } => {
                    assert_eq!(hostname.as_deref(), Some("beagle"));
                    assert!(usb_enable_dhcp);
                    assert!(file_destination);
                }
                other => panic!("expected Sd, got {other:?}"),
            },
            other => panic!("expected Flash, got {other:?}"),
        }
    }

    #[test]
    fn user_name_requires_password() {
        // `--user-name` declares `requires = "user_password"`.
        assert!(
            Opt::try_parse_from([
                "bb-imager-cli",
                "flash",
                "sd",
                "i",
                "/d",
                "--user-name",
                "bob",
            ])
            .is_err()
        );
        assert!(
            Opt::try_parse_from([
                "bb-imager-cli",
                "flash",
                "sd",
                "i",
                "/d",
                "--user-name",
                "bob",
                "--user-password",
                "pw",
            ])
            .is_ok()
        );
    }

    #[test]
    fn wifi_ssid_requires_password() {
        assert!(
            Opt::try_parse_from([
                "bb-imager-cli",
                "flash",
                "sd",
                "i",
                "/d",
                "--wifi-ssid",
                "net",
            ])
            .is_err()
        );
        assert!(
            Opt::try_parse_from([
                "bb-imager-cli",
                "flash",
                "sd",
                "i",
                "/d",
                "--wifi-ssid",
                "net",
                "--wifi-password",
                "pw",
            ])
            .is_ok()
        );
    }

    #[test]
    fn list_destinations_flags_parse() {
        let opt = Opt::try_parse_from([
            "bb-imager-cli",
            "list-destinations",
            "sd",
            "--no-frills",
            "--no-filter",
        ])
        .expect("valid list-destinations");
        match opt.command {
            Commands::ListDestinations {
                target,
                no_frills,
                no_filter,
            } => {
                assert!(matches!(target, DestinationsTarget::Sd));
                assert!(no_frills);
                assert!(no_filter);
            }
            other => panic!("expected ListDestinations, got {other:?}"),
        }
    }

    #[test]
    fn format_and_verbose_parse() {
        let opt = Opt::try_parse_from([
            "bb-imager-cli",
            "--verbose",
            "format",
            "/dev/sdX",
            "--quiet",
        ])
        .expect("valid format invocation");
        assert!(opt.verbose);
        match opt.command {
            Commands::Format { dst, quiet } => {
                assert_eq!(dst, PathBuf::from("/dev/sdX"));
                assert!(quiet);
            }
            other => panic!("expected Format, got {other:?}"),
        }
    }

    #[test]
    fn generate_completion_parses_shell() {
        let opt = Opt::try_parse_from(["bb-imager-cli", "generate-completion", "bash"])
            .expect("valid completion invocation");
        assert!(matches!(
            opt.command,
            Commands::GenerateCompletion {
                shell: clap_complete::Shell::Bash
            }
        ));
    }

    #[test]
    fn unknown_subcommand_is_rejected() {
        assert!(Opt::try_parse_from(["bb-imager-cli", "bogus"]).is_err());
    }

    #[test]
    fn sd_boot_update_is_no_longer_a_subcommand() {
        // The boot-archive path was removed with `SdCardBootfs`; the old
        // invocation must fail to parse rather than silently do something else.
        assert!(
            Opt::try_parse_from([
                "bb-imager-cli",
                "flash",
                "sd-boot-update",
                "boot.tar",
                "/dev/sdX",
            ])
            .is_err()
        );
    }
}
