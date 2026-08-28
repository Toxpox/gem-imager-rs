use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(version, about)]
pub struct Opt {
    #[command(subcommand)]
    pub command: Commands,
    #[arg(long)]
    pub verbose: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    Flash {
        #[command(subcommand)]
        target: Box<TargetCommands>,

        #[arg(long)]
        quiet: bool,
    },

    ListDestinations {
        target: DestinationsTarget,

        #[arg(long)]
        no_frills: bool,

        #[arg(long)]
        no_filter: bool,
    },

    Format {
        dst: PathBuf,

        #[arg(long)]
        quiet: bool,
    },

    GenerateCompletion {
        shell: clap_complete::Shell,
    },
}

#[derive(Subcommand, Debug)]
pub enum TargetCommands {
    Sd {
        img: Box<Path>,

        dst: PathBuf,

        #[arg(long)]
        hostname: Option<Box<str>>,

        #[arg(long)]
        timezone: Option<Box<str>>,

        #[arg(long)]
        keymap: Option<Box<str>>,

        #[arg(long, requires = "user_password", verbatim_doc_comment)]
        user_name: Option<Box<str>>,

        #[arg(long, requires = "user_name", verbatim_doc_comment)]
        user_password: Option<Box<str>>,

        #[arg(long, requires = "wifi_password")]
        wifi_ssid: Option<Box<str>>,

        #[arg(long, requires = "wifi_ssid")]
        wifi_password: Option<Box<str>>,

        #[arg(long)]
        ssh_key: Option<Box<str>>,

        #[arg(long)]
        usb_enable_dhcp: bool,

        #[arg(long)]
        cloud_init: bool,

        #[arg(long)]
        sysconfig: bool,

        #[arg(long)]
        file_destination: bool,
    },
    #[cfg(feature = "dfu")]
    Dfu {
        identifier: String,
        image: PathBuf,
        #[arg(long)]
        cache_dir: Option<PathBuf>,
    },
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum DestinationsTarget {
    Sd,
    #[cfg(feature = "dfu")]
    Dfu,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Opt::command().debug_assert();
    }

    #[test]
    fn flash_sd_minimal_parses() {
        let opt = Opt::try_parse_from(["gem-imager-cli", "flash", "sd", "img.xz", "/dev/sdX"])
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
                    #[cfg(feature = "dfu")]
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
            "gem-imager-cli",
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
    fn no_help_text_advertises_the_upstream_product() {
        fn assert_clean(cmd: &clap::Command) {
            let rendered = cmd.clone().render_long_help().to_string();
            assert!(
                !rendered.to_lowercase().contains("beagle"),
                "`{}` help still refers to the upstream product:\n{rendered}",
                cmd.get_name()
            );

            for sub in cmd.get_subcommands() {
                assert_clean(sub);
            }
        }

        assert_clean(&Opt::command());
    }

    #[test]
    fn flash_sd_customization_flags_parse() {
        let opt = Opt::try_parse_from([
            "gem-imager-cli",
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
                #[cfg(feature = "dfu")]
                other => panic!("expected Sd, got {other:?}"),
            },
            other => panic!("expected Flash, got {other:?}"),
        }
    }

    #[test]
    fn user_name_requires_password() {
        assert!(
            Opt::try_parse_from([
                "gem-imager-cli",
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
                "gem-imager-cli",
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
                "gem-imager-cli",
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
                "gem-imager-cli",
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
            "gem-imager-cli",
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
            "gem-imager-cli",
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
        let opt = Opt::try_parse_from(["gem-imager-cli", "generate-completion", "bash"])
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
        assert!(Opt::try_parse_from(["gem-imager-cli", "bogus"]).is_err());
    }

    #[test]
    fn sd_boot_update_is_no_longer_a_subcommand() {
        assert!(
            Opt::try_parse_from([
                "gem-imager-cli",
                "flash",
                "sd-boot-update",
                "boot.tar",
                "/dev/sdX",
            ])
            .is_err()
        );
    }
}
