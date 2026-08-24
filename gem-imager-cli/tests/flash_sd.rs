
use std::io::{Read, Seek, Write};

use clap::Parser;
use gem_flasher_sd::mock_sd::MockSd;
use gem_imager_cli::cli::Opt;
use tempfile::NamedTempFile;

fn run_cli<const N: usize>(args: [&str; N]) {
    let opt = Opt::try_parse_from(args).expect("argv should parse");
    gem_imager_cli::run(opt);
}

fn pattern_file(len: usize) -> NamedTempFile {
    let data: Vec<u8> = (0..len).map(|x| (x % 251) as u8).collect();
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(&data).unwrap();
    f.flush().unwrap();
    f
}

struct SdFixture {
    mock: MockSd,
    image: NamedTempFile,
}

impl SdFixture {
    fn new() -> Self {
        Self::with_boot_dirs(&[])
    }

    fn with_boot_dirs(dirs: &[&str]) -> Self {
        let mut mock = MockSd::new();

        if !dirs.is_empty() {
            let fs = mock.open_boot();
            for dir in dirs {
                fs.root_dir().create_dir(dir).unwrap();
            }
            fs.unmount().unwrap();
        }

        let mut image = NamedTempFile::new().unwrap();
        let mut src = std::fs::File::open(mock.path()).unwrap();
        std::io::copy(&mut src, image.as_file_mut()).unwrap();
        image.flush().unwrap();

        Self { mock, image }
    }

    fn img(&self) -> &str {
        self.image.path().to_str().unwrap()
    }

    fn dst(&self) -> &str {
        self.mock.path().to_str().unwrap()
    }

    fn boot_file(&mut self, name: &str) -> std::io::Result<String> {
        self.mock.rewind().unwrap();
        let fs = self.mock.open_boot();
        let mut out = String::new();
        fs.root_dir()
            .open_file(name)
            .map_err(std::io::Error::other)?
            .read_to_string(&mut out)?;
        Ok(out)
    }
}

fn read_all(path: &std::path::Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

#[test]
fn flash_sd_file_destination_copies_image_verbatim() {
    let img = pattern_file(64 * 1024);
    let dst = NamedTempFile::new().unwrap();

    run_cli([
        "gem-imager-cli",
        "flash",
        "--quiet",
        "sd",
        img.path().to_str().unwrap(),
        dst.path().to_str().unwrap(),
        "--file-destination",
    ]);

    assert_eq!(read_all(dst.path()), read_all(img.path()));
}

#[test]
fn flash_sd_renders_progress_when_not_quiet() {
    let img = pattern_file(64 * 1024);
    let dst = NamedTempFile::new().unwrap();

    run_cli([
        "gem-imager-cli",
        "flash",
        "sd",
        img.path().to_str().unwrap(),
        dst.path().to_str().unwrap(),
        "--file-destination",
    ]);

    assert_eq!(read_all(dst.path()), read_all(img.path()));
}

#[test]
fn flash_sd_creates_missing_destination_file() {
    let img = pattern_file(4 * 1024);
    let dir = tempfile::tempdir().unwrap();
    let dst = dir.path().join("out.img");

    run_cli([
        "gem-imager-cli",
        "flash",
        "--quiet",
        "sd",
        img.path().to_str().unwrap(),
        dst.to_str().unwrap(),
        "--file-destination",
    ]);

    assert_eq!(read_all(&dst), read_all(img.path()));
}

#[test]
fn flash_sd_without_customization_flags_writes_no_config() {
    let mut fixture = SdFixture::new();

    run_cli([
        "gem-imager-cli",
        "flash",
        "--quiet",
        "sd",
        fixture.img(),
        fixture.dst(),
        "--file-destination",
    ]);

    assert!(
        fixture.boot_file("sysconf.txt").is_err(),
        "sysconf.txt should not be created without customization flags"
    );
}

#[test]
fn flash_sd_sysconfig_writes_every_supplied_key() {
    let mut fixture = SdFixture::new();

    run_cli([
        "gem-imager-cli",
        "flash",
        "--quiet",
        "sd",
        fixture.img(),
        fixture.dst(),
        "--file-destination",
        "--sysconfig",
        "--hostname",
        "beagle",
        "--timezone",
        "Asia/Kolkata",
        "--keymap",
        "us",
        "--user-name",
        "bob",
        "--user-password",
        "hunter2",
        "--ssh-key",
        "ssh-ed25519 AAAA",
        "--usb-enable-dhcp",
    ]);

    assert_eq!(
        fixture.boot_file("sysconf.txt").unwrap(),
        "hostname=beagle\n\
         timezone=Asia/Kolkata\n\
         keymap=us\n\
         user_name=bob\n\
         user_password=hunter2\n\
         user_authorized_key=ssh-ed25519 AAAA\n\
         usb_enable_dhcp=yes\n"
    );
}

#[test]
fn flash_sd_sysconfig_omits_unset_keys() {
    let mut fixture = SdFixture::new();

    run_cli([
        "gem-imager-cli",
        "flash",
        "--quiet",
        "sd",
        fixture.img(),
        fixture.dst(),
        "--file-destination",
        "--sysconfig",
        "--hostname",
        "beagle",
    ]);

    assert_eq!(
        fixture.boot_file("sysconf.txt").unwrap(),
        "hostname=beagle\n"
    );
}

#[test]
fn flash_sd_without_usb_dhcp_flag_omits_the_key() {
    let mut fixture = SdFixture::new();

    run_cli([
        "gem-imager-cli",
        "flash",
        "--quiet",
        "sd",
        fixture.img(),
        fixture.dst(),
        "--file-destination",
        "--sysconfig",
        "--keymap",
        "us",
    ]);

    assert_eq!(fixture.boot_file("sysconf.txt").unwrap(), "keymap=us\n");
}

/// NOTE: `services/` must already exist in the image's boot partition — the
/// customization writer uses `create_file`, which does not create parent
/// directories, so `--wifi-ssid` on an image without that directory fails the
/// whole flash with "Failed to create customization services/<ssid>.psk".
#[test]
fn flash_sd_wifi_writes_psk_file_next_to_sysconfig() {
    let mut fixture = SdFixture::with_boot_dirs(&["services"]);

    run_cli([
        "gem-imager-cli",
        "flash",
        "--quiet",
        "sd",
        fixture.img(),
        fixture.dst(),
        "--file-destination",
        "--sysconfig",
        "--wifi-ssid",
        "mynet",
        "--wifi-password",
        "hunter2",
    ]);

    assert_eq!(
        fixture.boot_file("sysconf.txt").unwrap(),
        "iwd_psk_file=mynet.psk\n"
    );
    assert_eq!(
        fixture.boot_file("services/mynet.psk").unwrap(),
        "[Security]\nPassphrase=hunter2\n\n[Settings]\nAutoConnect=true"
    );
}

#[test]
fn flash_sd_cloud_init_emits_both_configs() {
    let mut fixture = SdFixture::new();

    run_cli([
        "gem-imager-cli",
        "flash",
        "--quiet",
        "sd",
        fixture.img(),
        fixture.dst(),
        "--file-destination",
        "--cloud-init",
        "--hostname",
        "beagle",
        "--user-name",
        "bob",
        "--user-password",
        "hunter2",
    ]);

    let cloud_init = fixture.boot_file("cloud-init").unwrap();
    assert!(
        cloud_init.starts_with("#cloud-config\n"),
        "cloud-init must carry the cloud-config header, got: {cloud_init}"
    );
    assert!(
        cloud_init.contains("beagle"),
        "cloud-init should carry the hostname, got: {cloud_init}"
    );

    assert!(
        fixture.boot_file("sysconf.txt").is_ok(),
        "sysconfig is still generated alongside cloud-init"
    );
}

#[test]
#[should_panic(expected = "Failed to flash")]
fn flash_sd_customization_on_partitionless_image_fails() {
    let img = pattern_file(64 * 1024);
    let dst = NamedTempFile::new().unwrap();

    run_cli([
        "gem-imager-cli",
        "flash",
        "--quiet",
        "sd",
        img.path().to_str().unwrap(),
        dst.path().to_str().unwrap(),
        "--file-destination",
        "--sysconfig",
        "--hostname",
        "beagle",
    ]);
}

#[test]
#[should_panic(expected = "Failed to flash")]
fn flash_sd_missing_image_fails() {
    let dst = NamedTempFile::new().unwrap();

    run_cli([
        "gem-imager-cli",
        "flash",
        "--quiet",
        "sd",
        "/nonexistent/image.img",
        dst.path().to_str().unwrap(),
        "--file-destination",
    ]);
}
