use std::process::Command;

use crate::device::{DeviceDescriptor, MountPoint};
use serde::Deserialize;

const OS_MOUNTS: &[&str] = &[
    "/",
    "/boot",
    "/boot/efi",
    "/efi",
    "/usr",
    "/var",
    "/etc",
    "/nix",
];

#[derive(Debug, Default)]
pub(crate) struct OsMounts {
    mounts: Vec<String>,
}

impl OsMounts {
    pub(crate) fn current() -> Self {
        Self::from_mountinfo(&std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default())
    }

    fn from_mountinfo(mountinfo: &str) -> Self {
        let mounts = mountinfo
            .lines()
            .filter_map(|line| line.split_whitespace().nth(4))
            .filter(|mount| OS_MOUNTS.contains(mount))
            .map(str::to_owned)
            .collect();

        Self { mounts }
    }

    fn claims(&self, mount: &str) -> bool {
        if mount == "[SWAP]" {
            return true;
        }

        self.mounts.iter().any(|os_mount| os_mount == mount)
    }
}

#[derive(Deserialize, Debug)]
struct Devices {
    blockdevices: Vec<Device>,
}

#[derive(Deserialize, Debug)]
struct Device {
    size: Option<u64>,
    #[serde(default = "Device::name_default")]
    kname: String,
    #[serde(default = "Device::name_default")]
    name: String,
    tran: Option<String>,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    subsystems: Option<String>,
    ro: bool,
    #[serde(rename = "phy-sec")]
    phy_sec: u32,
    #[serde(rename = "log-sec")]
    log_sec: u32,
    rm: bool,
    pttype: Option<String>,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    serial: Option<String>,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    wwn: Option<String>,
    #[serde(default)]
    children: Vec<Child>,
    mountpoint: Option<String>,
    fssize: Option<FsSize>,
    fsavail: Option<FsSize>,
    label: Option<String>,
    vendor: Option<String>,
    model: Option<String>,
    hotplug: bool,
}

impl Device {
    fn name_default() -> String {
        "NO_NAME".to_string()
    }

    fn is_scsi(&self) -> bool {
        self.subsystems.as_ref().is_some_and(|x| {
            x.contains("sata")
                || x.contains("scsi")
                || x.contains("ata")
                || x.contains("ide")
                || x.contains("pci")
        })
    }

    fn description(&self) -> String {
        [
            self.label.as_deref().unwrap_or_default(),
            self.vendor.as_deref().unwrap_or_default(),
            self.model.as_deref().unwrap_or_default(),
        ]
        .into_iter()
        .filter(|x| !x.is_empty())
        .fold(String::new(), |mut acc, x| {
            acc.push_str(x);
            acc
        })
    }

    fn is_virtual(&self) -> bool {
        self.subsystems
            .as_ref()
            .is_some_and(|x| !x.contains("block"))
    }

    fn is_removable(&self) -> bool {
        self.rm || self.hotplug || self.is_virtual()
    }

    fn is_system(&self) -> bool {
        !(self.is_removable() || self.is_virtual())
    }

    fn holds_os_mount(&self, os_mounts: &OsMounts) -> bool {
        std::iter::once(self.mountpoint.as_deref())
            .chain(self.children.iter().flat_map(Child::mountpoints))
            .flatten()
            .any(|mount| os_mounts.claims(mount))
    }

    fn mountpoints(self) -> Vec<MountPoint> {
        let whole_disk = self
            .mountpoint
            .filter(|path| !path.is_empty())
            .map(|path| MountPoint {
                path,
                label: self.label.clone(),
                total_bytes: self.fssize.map(Into::into),
                available_bytes: self.fsavail.map(Into::into),
            });

        whole_disk
            .into_iter()
            .chain(self.children.into_iter().flat_map(Child::into_mountpoints))
            .collect()
    }
}

impl Device {
    fn into_descriptor(mut self, os_mounts: &OsMounts) -> DeviceDescriptor {
        let value = &mut self;
        let is_scsi = value.is_scsi();
        let description = value.description();
        let is_virtual = value.is_virtual();
        let is_removable = value.is_removable();
        let is_system = value.is_system() || value.holds_os_mount(os_mounts);
        let is_usb = value
            .subsystems
            .as_deref()
            .is_some_and(|x| x.contains("usb"));
        let bus_type = Some(value.tran.as_deref().unwrap_or("UNKNOWN").to_uppercase());
        let name = std::mem::take(&mut value.name);
        let kname = std::mem::take(&mut value.kname);
        let size = value.size;
        let is_readonly = value.ro;
        let block_size = value.phy_sec;
        let logical_block_size = value.log_sec;
        let partition_table_type = value.pttype.take();
        let serial = value.serial.take();
        let wwn = value.wwn.take();
        let mountpoints = self.mountpoints();

        DeviceDescriptor {
            enumerator: "lsblk:json".to_string(),
            bus_type,
            device: name,
            raw: kname,
            is_virtual,
            is_scsi,
            is_usb,
            is_readonly,
            description,
            size,
            block_size,
            logical_block_size,
            is_removable,
            is_system,
            partition_table_type,
            mountpoints,
            serial,
            wwn,
            ..Default::default()
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
enum FsSize {
    String(String),
    U64(u64),
}

impl From<FsSize> for u64 {
    fn from(value: FsSize) -> Self {
        match value {
            FsSize::String(x) => x.parse().unwrap(),
            FsSize::U64(x) => x,
        }
    }
}

#[derive(Deserialize, Debug)]
struct Child {
    mountpoint: Option<String>,
    fssize: Option<FsSize>,
    fsavail: Option<FsSize>,
    label: Option<String>,
    partlabel: Option<String>,
    #[serde(default)]
    children: Vec<Child>,
}

impl Child {
    fn mountpoints(&self) -> impl Iterator<Item = Option<&str>> {
        std::iter::once(self.mountpoint.as_deref())
            .chain(self.descendants().map(|c| c.mountpoint.as_deref()))
    }

    fn descendants(&self) -> Box<dyn Iterator<Item = &Self> + '_> {
        Box::new(
            self.children
                .iter()
                .flat_map(|child| std::iter::once(child).chain(child.descendants())),
        )
    }

    fn into_mountpoints(self) -> Vec<MountPoint> {
        let children = self.children;
        let own = MountPoint {
            path: self.mountpoint.unwrap_or_default(),
            label: if self.label.is_some() {
                self.label
            } else {
                self.partlabel
            },
            total_bytes: self.fssize.map(Into::into),
            available_bytes: self.fsavail.map(Into::into),
        };

        std::iter::once(own)
            .chain(children.into_iter().flat_map(Self::into_mountpoints))
            .collect()
    }
}

impl From<Child> for MountPoint {
    fn from(value: Child) -> Self {
        Self {
            path: value.mountpoint.unwrap_or_default(),
            label: if value.label.is_some() {
                value.label
            } else {
                value.partlabel
            },
            total_bytes: value.fssize.map(Into::into),
            available_bytes: value.fsavail.map(Into::into),
        }
    }
}

const COLUMNS: &str = "NAME,KNAME,SIZE,TRAN,SUBSYSTEMS,RO,RM,HOTPLUG,PHY-SEC,LOG-SEC,\
                       PTTYPE,LABEL,VENDOR,MODEL,MOUNTPOINT,FSSIZE,FSAVAIL,PARTLABEL,\
                       SERIAL,WWN";

pub(crate) fn lsblk() -> crate::Result<Vec<DeviceDescriptor>> {
    let output = Command::new("lsblk")
        .args(["--bytes", "--all", "--json", "--paths", "--output", COLUMNS])
        .output()
        .map_err(|e| crate::Error::LsblkExecuteError { source: Some(e) })?;

    if !output.status.success() {
        return Err(crate::Error::LsblkExecuteError { source: None });
    }

    let res: Devices = serde_json::from_slice(&output.stdout).unwrap();

    let os_mounts = OsMounts::current();

    Ok(res
        .blockdevices
        .into_iter()
        .map(|device| device.into_descriptor(&os_mounts))
        .collect())
}

fn empty_string_as_none<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.filter(|s| !s.is_empty()))
}

#[cfg(test)]
mod tests {
    use crate::DeviceDescriptor;

    #[test]
    fn loop_dev() {
        let data = r#"
        {
            "blockdevices": [
                {
                    "name":"/dev/loop23", 
                    "kname":"/dev/loop23", 
                    "path":"/dev/loop23", 
                    "maj:min":"7:23", 
                    "fsavail":null, 
                    "fssize":null, 
                    "fstype":null, 
                    "fsused":null, 
                    "fsuse%":null, 
                    "mountpoint":null, 
                    "label":null, 
                    "uuid":null, 
                    "ptuuid":null, 
                    "pttype":null, 
                    "parttype":null, 
                    "partlabel":null, 
                    "partuuid":null, 
                    "partflags":null, 
                    "ra":128, 
                    "ro":false, 
                    "rm":false, 
                    "hotplug":false, 
                    "model":null, 
                    "serial":null, 
                    "size":null, 
                    "state":null, 
                    "owner":"root", 
                    "group":"disk", 
                    "mode":"brw-rw----", 
                    "alignment":0, 
                    "min-io":512, 
                    "opt-io":0, 
                    "phy-sec":512, 
                    "log-sec":512, 
                    "rota":false, 
                    "sched":"none", 
                    "rq-size":128, 
                    "type":"loop", 
                    "disc-aln":0, 
                    "disc-gran":4096, 
                    "disc-max":4294966784, 
                    "disc-zero":false, 
                    "wsame":0, 
                    "wwn":null, 
                    "rand":false, 
                    "pkname":null, 
                    "hctl":null, 
                    "tran":null, 
                    "subsystems":"block", 
                    "rev":null, 
                    "vendor":null, 
                    "zoned":"none"
                }
            ]
        }"#;

        let res: super::Devices = serde_json::from_str(data).unwrap();
        let os_mounts = super::OsMounts::default();
        let _: Vec<DeviceDescriptor> = res
            .blockdevices
            .into_iter()
            .map(|device| device.into_descriptor(&os_mounts))
            .collect();
    }

    fn descriptors(blockdevices: &str) -> Vec<DeviceDescriptor> {
        descriptors_with_os_mounts(blockdevices, &super::OsMounts::default())
    }

    fn descriptors_with_os_mounts(
        blockdevices: &str,
        os_mounts: &super::OsMounts,
    ) -> Vec<DeviceDescriptor> {
        let data = format!(r#"{{"blockdevices":{blockdevices}}}"#);
        let res: super::Devices = serde_json::from_str(&data).unwrap();
        res.blockdevices
            .into_iter()
            .map(|device| device.into_descriptor(os_mounts))
            .collect()
    }

    #[test]
    fn usb_removable_disk_classification() {
        let d = &descriptors(
            r#"[{
                "name":"/dev/sda","kname":"/dev/sda",
                "size":32000000000,"tran":"usb",
                "subsystems":"block:scsi:usb:pci","ro":false,
                "phy-sec":512,"log-sec":512,"rm":true,"hotplug":false,
                "pttype":"gpt","label":"BOOT","vendor":"Kingston","model":"DataTraveler"
            }]"#,
        )[0];

        assert_eq!(d.enumerator, "lsblk:json");
        assert_eq!(d.device, "/dev/sda");
        assert_eq!(d.raw, "/dev/sda");
        assert_eq!(d.bus_type.as_deref(), Some("USB"));
        assert!(d.is_usb);
        assert!(d.is_scsi);
        assert!(!d.is_virtual);
        assert!(d.is_removable);
        assert!(!d.is_system);
        assert!(!d.is_readonly);
        assert_eq!(d.size, Some(32000000000));
        assert_eq!(d.block_size, 512);
        assert_eq!(d.logical_block_size, 512);
        assert_eq!(d.partition_table_type.as_deref(), Some("gpt"));
        assert_eq!(d.description, "BOOTKingstonDataTraveler");
    }

    #[test]
    fn internal_disk_is_system_not_removable() {
        let d = &descriptors(
            r#"[{
                "name":"/dev/nvme0n1","kname":"/dev/nvme0n1",
                "size":512000000000,"tran":"nvme",
                "subsystems":"block:nvme:pci","ro":false,
                "phy-sec":512,"log-sec":4096,"rm":false,"hotplug":false,
                "pttype":null,"label":null,"vendor":null,"model":"Samsung SSD"
            }]"#,
        )[0];

        assert_eq!(d.bus_type.as_deref(), Some("NVME"));
        assert!(!d.is_usb);
        assert!(d.is_scsi);
        assert!(!d.is_virtual);
        assert!(!d.is_removable);
        assert!(d.is_system);
        assert_eq!(d.logical_block_size, 4096);
        assert_eq!(d.description, "Samsung SSD");
        assert_eq!(d.partition_table_type, None);
    }

    #[test]
    fn virtual_device_without_block_subsystem() {
        let d = &descriptors(
            r#"[{
                "name":"/dev/dm-0","kname":"/dev/dm-0",
                "size":null,"tran":null,
                "subsystems":"nvme:pci",
                "ro":true,
                "phy-sec":512,"log-sec":512,"rm":false,"hotplug":false,
                "pttype":null,"label":null,"vendor":null,"model":null
            }]"#,
        )[0];

        assert!(d.is_virtual);
        assert!(d.is_removable);
        assert!(!d.is_system);
        assert_eq!(d.bus_type.as_deref(), Some("UNKNOWN"));
        assert!(d.is_readonly);
        assert!(!d.is_usb);
        assert_eq!(d.description, "");
    }

    #[test]
    fn hotplug_alone_marks_removable() {
        let d = &descriptors(
            r#"[{
                "name":"/dev/sdb","kname":"/dev/sdb",
                "size":8000000000,"tran":"usb",
                "subsystems":"block:usb","ro":false,
                "phy-sec":512,"log-sec":512,"rm":false,"hotplug":true,
                "pttype":null,"label":null,"vendor":null,"model":null
            }]"#,
        )[0];

        assert!(d.is_removable);
        assert!(!d.is_system);
        assert!(d.is_usb);
        assert!(!d.is_virtual);
    }

    #[test]
    fn children_map_to_mountpoints_with_fssize_variants() {
        let d = &descriptors(
            r#"[{
                "name":"/dev/sdc","kname":"/dev/sdc",
                "size":16000000000,"tran":"usb",
                "subsystems":"block:usb","ro":false,
                "phy-sec":512,"log-sec":512,"rm":true,"hotplug":false,
                "pttype":null,"label":null,"vendor":null,"model":null,
                "children":[
                    {"mountpoint":"/boot","fssize":"1048576","fsavail":524288,"label":null,"partlabel":"BOOTFS"},
                    {"mountpoint":null,"fssize":null,"fsavail":null,"label":"ROOT","partlabel":"rootfs"}
                ]
            }]"#,
        )[0];

        assert_eq!(d.mountpoints.len(), 2);

        assert_eq!(d.mountpoints[0].path, "/boot");
        assert_eq!(d.mountpoints[0].total_bytes, Some(1048576));
        assert_eq!(d.mountpoints[0].available_bytes, Some(524288));
        assert_eq!(d.mountpoints[0].label.as_deref(), Some("BOOTFS"));

        assert_eq!(d.mountpoints[1].path, "");
        assert_eq!(d.mountpoints[1].total_bytes, None);
        assert_eq!(d.mountpoints[1].label.as_deref(), Some("ROOT"));
    }

    #[test]
    fn missing_name_uses_default() {
        let d = &descriptors(
            r#"[{
                "size":null,"tran":null,
                "subsystems":"block","ro":false,
                "phy-sec":512,"log-sec":512,"rm":false,"hotplug":false,
                "pttype":null,"label":null,"vendor":null,"model":null
            }]"#,
        )[0];

        assert_eq!(d.device, "NO_NAME");
        assert_eq!(d.raw, "NO_NAME");
    }

    #[test]
    fn null_subsystems_parses_and_classifies() {
        let d = &descriptors(
            r#"[{
                "name":"/dev/sdd","kname":"/dev/sdd",
                "size":64000000000,"tran":"usb",
                "subsystems":null,"ro":false,
                "phy-sec":512,"log-sec":512,"rm":false,"hotplug":false,
                "pttype":null,"label":null,"vendor":null,"model":"Generic"
            }]"#,
        )[0];

        assert!(!d.is_scsi);
        assert!(!d.is_usb);
        assert!(!d.is_virtual);
        assert!(!d.is_removable);
        assert!(d.is_system);
        assert_eq!(d.bus_type.as_deref(), Some("USB"));
        assert_eq!(d.description, "Generic");
    }

    #[test]
    fn omitted_subsystems_key_parses() {
        let d = &descriptors(
            r#"[{
                "name":"/dev/sde","kname":"/dev/sde",
                "size":null,"tran":null,"ro":false,
                "phy-sec":512,"log-sec":512,"rm":false,"hotplug":false,
                "pttype":null,"label":null,"vendor":null,"model":null
            }]"#,
        )[0];

        assert!(!d.is_scsi);
        assert!(!d.is_usb);
        assert!(!d.is_virtual);
        assert_eq!(d.bus_type.as_deref(), Some("UNKNOWN"));
    }

    #[test]
    fn null_subsystems_is_not_virtual() {
        let d = &descriptors(
            r#"[{
                "name":"/dev/dm-1","kname":"/dev/dm-1",
                "size":null,"tran":null,"subsystems":null,"ro":false,
                "phy-sec":512,"log-sec":512,"rm":false,"hotplug":false,
                "pttype":null,"label":null,"vendor":null,"model":null
            }]"#,
        )[0];

        assert!(!d.is_virtual);
        assert!(!d.is_removable);
        assert!(d.is_system);
    }

    const REMOVABLE_BOOT_DISK: &str = r#"[{
        "name":"/dev/sda","kname":"/dev/sda",
        "size":32000000000,"tran":"usb","subsystems":"block:scsi:usb","ro":false,
        "phy-sec":512,"log-sec":512,"rm":true,"hotplug":true,
        "pttype":"gpt","label":null,"vendor":null,"model":null,
        "mountpoint":null,
        "children":[
            {"mountpoint":"/boot","label":"BOOT"},
            {"mountpoint":"/","label":"ROOT"}
        ]
    }]"#;

    #[test]
    fn a_removable_disk_carrying_the_running_system_is_not_offered_as_a_target() {
        let os_mounts = super::OsMounts::from_mountinfo(
            "31 1 259:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw\n             32 31 259:1 / /boot rw,relatime shared:2 - vfat /dev/sda1 rw\n",
        );
        let d = &descriptors_with_os_mounts(REMOVABLE_BOOT_DISK, &os_mounts)[0];

        assert!(
            d.is_system,
            "a USB disk holding / and /boot must be treated as the system disk"
        );
    }

    #[test]
    fn a_removable_disk_whose_root_sits_behind_dm_crypt_is_not_offered_as_a_target() {
        let os_mounts = super::OsMounts::from_mountinfo(
            "31 1 254:0 / / rw,relatime shared:1 - ext4 /dev/mapper/cryptroot rw\n",
        );
        let d = &descriptors_with_os_mounts(
            r#"[{
                "name":"/dev/sda","kname":"/dev/sda",
                "size":32000000000,"tran":"usb","subsystems":"block:scsi:usb","ro":false,
                "phy-sec":512,"log-sec":512,"rm":true,"hotplug":true,
                "pttype":"gpt","label":null,"vendor":null,"model":null,
                "mountpoint":null,
                "children":[{
                    "mountpoint":null,"label":null,
                    "children":[{"mountpoint":"/","label":"cryptroot"}]
                }]
            }]"#,
            &os_mounts,
        )[0];

        assert!(
            d.is_system,
            "a LUKS or LVM disk whose root is one level deeper must still be refused"
        );
    }

    #[test]
    fn mountpoints_below_the_first_child_level_are_reported() {
        let d = &descriptors(
            r#"[{
                "name":"/dev/sdb","kname":"/dev/sdb",
                "size":1000,"tran":"usb","subsystems":"block:scsi:usb","ro":false,
                "phy-sec":512,"log-sec":512,"rm":true,"hotplug":true,
                "pttype":"gpt","label":null,"vendor":null,"model":null,
                "mountpoint":null,
                "children":[{
                    "mountpoint":null,"label":null,
                    "children":[{"mountpoint":"/media/vault","label":"VAULT"}]
                }]
            }]"#,
        )[0];

        let paths: Vec<&str> = d
            .mountpoints
            .iter()
            .map(|m| m.path.as_str())
            .filter(|p| !p.is_empty())
            .collect();
        assert_eq!(
            paths,
            ["/media/vault"],
            "a nested mount must be unmounted before the raw write"
        );
    }

    #[test]
    fn a_removable_disk_without_os_mounts_stays_writable() {
        let os_mounts = super::OsMounts::from_mountinfo(
            "31 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw\n",
        );
        let d = &descriptors_with_os_mounts(
            r#"[{
                "name":"/dev/sdb","kname":"/dev/sdb",
                "size":32000000000,"tran":"usb","subsystems":"block:scsi:usb","ro":false,
                "phy-sec":512,"log-sec":512,"rm":true,"hotplug":true,
                "pttype":"dos","label":null,"vendor":null,"model":null,
                "mountpoint":null,
                "children":[{"mountpoint":"/media/card","label":"BOOT"}]
            }]"#,
            &os_mounts,
        )[0];

        assert!(
            !d.is_system,
            "an ordinary SD card must stay selectable as a flash target"
        );
    }

    #[test]
    fn a_disk_holding_swap_is_treated_as_a_system_disk() {
        let d = &descriptors(
            r#"[{
                "name":"/dev/sdc","kname":"/dev/sdc",
                "size":32000000000,"tran":"usb","subsystems":"block:scsi:usb","ro":false,
                "phy-sec":512,"log-sec":512,"rm":true,"hotplug":true,
                "pttype":"gpt","label":null,"vendor":null,"model":null,
                "mountpoint":null,
                "children":[{"mountpoint":"[SWAP]","label":null}]
            }]"#,
        )[0];

        assert!(d.is_system, "erasing active swap would crash the host");
    }

    #[test]
    fn os_mounts_ignores_unrelated_mount_points() {
        let os_mounts = super::OsMounts::from_mountinfo(
            "31 1 259:2 / /media/usb rw,relatime shared:1 - ext4 /dev/sdb1 rw\n             32 31 259:1 / /home rw,relatime shared:2 - ext4 /dev/sdb2 rw\n",
        );

        assert!(!os_mounts.claims("/media/usb"));
        assert!(!os_mounts.claims("/home"));
        assert!(!os_mounts.claims("/"));
    }

    #[test]
    fn a_filesystem_mounted_on_the_whole_disk_is_reported() {
        let d = &descriptors(
            r#"[{
                "name":"/dev/sdb","kname":"/dev/sdb",
                "size":1000,"tran":"usb","subsystems":"block:scsi:usb","ro":false,
                "phy-sec":512,"log-sec":512,"rm":true,"hotplug":true,
                "pttype":null,"label":"DATA","vendor":null,"model":null,
                "mountpoint":"/mnt/stick","fssize":900,"fsavail":400
            }]"#,
        )[0];

        assert_eq!(
            d.mountpoints.len(),
            1,
            "a partitionless mounted disk must not look unmounted"
        );
        assert_eq!(d.mountpoints[0].path, "/mnt/stick");
    }

    #[test]
    fn whole_disk_and_partition_mounts_are_both_reported() {
        let d = &descriptors(
            r#"[{
                "name":"/dev/sdb","kname":"/dev/sdb",
                "size":1000,"tran":"usb","subsystems":"block:scsi:usb","ro":false,
                "phy-sec":512,"log-sec":512,"rm":true,"hotplug":true,
                "pttype":"dos","label":null,"vendor":null,"model":null,
                "mountpoint":"/mnt/whole",
                "children":[{"mountpoint":"/mnt/part","label":"BOOT"}]
            }]"#,
        )[0];

        let paths: Vec<&str> = d.mountpoints.iter().map(|m| m.path.as_str()).collect();
        assert_eq!(paths, ["/mnt/whole", "/mnt/part"]);
    }

    #[test]
    fn an_unmounted_disk_reports_no_mountpoints() {
        let d = &descriptors(
            r#"[{
                "name":"/dev/sdb","kname":"/dev/sdb",
                "size":1000,"tran":"usb","subsystems":"block:scsi:usb","ro":false,
                "phy-sec":512,"log-sec":512,"rm":true,"hotplug":true,
                "pttype":null,"label":null,"vendor":null,"model":null,
                "mountpoint":null
            }]"#,
        )[0];

        assert!(d.mountpoints.is_empty());
    }

    #[test]
    fn empty_subsystems_normalized_to_none() {
        let d = &descriptors(
            r#"[{
                "name":"/dev/dm-2","kname":"/dev/dm-2",
                "size":null,"tran":null,"subsystems":"","ro":false,
                "phy-sec":512,"log-sec":512,"rm":false,"hotplug":false,
                "pttype":null,"label":null,"vendor":null,"model":null
            }]"#,
        )[0];

        assert!(!d.is_virtual);
        assert!(!d.is_scsi);
        assert!(!d.is_usb);
    }
}
