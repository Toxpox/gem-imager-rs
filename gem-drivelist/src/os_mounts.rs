const OS_MOUNTS: &[&str] = &[
    "/",
    "/boot",
    "/boot/efi",
    "/efi",
    "/usr",
    "/var",
    "/etc",
    "/nix",
    "/System/Volumes/Data",
    "/System/Volumes/Preboot",
    "/System/Volumes/VM",
    "/System/Volumes/Update",
];

#[derive(Debug, Default, Clone)]
pub(crate) struct OsMounts {
    mounts: Vec<String>,
}

impl OsMounts {
    #[cfg(target_os = "linux")]
    pub(crate) fn current() -> Self {
        Self::from_mountinfo(&std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default())
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn from_mountinfo(mountinfo: &str) -> Self {
        Self::from_paths(
            mountinfo
                .lines()
                .filter_map(|line| line.split_whitespace().nth(4)),
        )
    }

    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) fn from_paths<'a>(paths: impl IntoIterator<Item = &'a str>) -> Self {
        Self {
            mounts: paths
                .into_iter()
                .filter(|mount| is_os_mount(mount))
                .map(str::to_owned)
                .collect(),
        }
    }

    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) fn claims(&self, mount: &str) -> bool {
        if mount == "[SWAP]" {
            return true;
        }

        self.mounts.iter().any(|os_mount| os_mount == mount)
    }
}

pub(crate) fn is_os_mount(mount: &str) -> bool {
    OS_MOUNTS.contains(&mount)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_root_and_its_boot_partitions_belong_to_the_operating_system() {
        for mount in ["/", "/boot", "/boot/efi", "/efi", "/usr", "/var", "/etc"] {
            assert!(is_os_mount(mount), "{mount} carries the running system");
        }
    }

    #[test]
    fn the_macos_system_volumes_belong_to_the_operating_system() {
        for mount in [
            "/System/Volumes/Data",
            "/System/Volumes/Preboot",
            "/System/Volumes/VM",
            "/System/Volumes/Update",
        ] {
            assert!(
                is_os_mount(mount),
                "{mount} is part of a running macOS install"
            );
        }
    }

    #[test]
    fn ordinary_media_mounts_are_not_system_mounts() {
        for mount in ["/media/usb", "/mnt/card", "/Volumes/UNTITLED", "/home"] {
            assert!(!is_os_mount(mount), "{mount} must stay a writable target");
        }
    }

    #[test]
    fn active_swap_is_always_claimed() {
        assert!(OsMounts::default().claims("[SWAP]"));
    }

    #[test]
    fn only_declared_os_mounts_are_claimed() {
        let mounts = OsMounts::from_paths(["/", "/media/usb"]);

        assert!(mounts.claims("/"));
        assert!(!mounts.claims("/media/usb"));
        assert!(!mounts.claims("/boot"));
    }
}
