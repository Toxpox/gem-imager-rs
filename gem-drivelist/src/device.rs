#[derive(Debug, Default, Clone)]
pub struct MountPoint {
    pub path: String,
    pub label: Option<String>,
    pub total_bytes: Option<u64>,
    pub available_bytes: Option<u64>,
}

impl MountPoint {
    pub fn new(path: impl ToString) -> Self {
        Self {
            path: path.to_string(),
            label: None,
            total_bytes: None,
            available_bytes: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeviceDescriptor {
    pub enumerator: String,
    pub bus_type: Option<String>,
    pub bus_version: Option<String>,
    pub device: String,
    pub device_path: Option<String>,
    pub raw: String,
    pub description: String,
    pub error: Option<String>,
    pub partition_table_type: Option<String>,
    pub size: Option<u64>,
    pub block_size: u32,
    pub logical_block_size: u32,
    pub mountpoints: Vec<MountPoint>,
    pub mountpoint_labels: Vec<String>,
    pub is_readonly: bool,
    pub is_system: bool,
    pub is_card: bool,
    pub is_scsi: bool,
    pub is_usb: bool,
    pub is_virtual: bool,
    pub is_removable: bool,
    pub is_uas: Option<bool>,
    pub serial: Option<String>,
    pub wwn: Option<String>,
}

impl Default for DeviceDescriptor {
    fn default() -> Self {
        Self {
            block_size: 512,
            logical_block_size: 512,
            enumerator: Default::default(),
            bus_type: Default::default(),
            bus_version: Default::default(),
            device: Default::default(),
            device_path: Default::default(),
            raw: Default::default(),
            description: Default::default(),
            error: Default::default(),
            partition_table_type: Default::default(),
            size: Default::default(),
            mountpoints: Default::default(),
            mountpoint_labels: Default::default(),
            is_readonly: Default::default(),
            is_system: Default::default(),
            is_card: Default::default(),
            is_scsi: Default::default(),
            is_usb: Default::default(),
            is_virtual: Default::default(),
            is_removable: Default::default(),
            is_uas: Default::default(),
            serial: Default::default(),
            wwn: Default::default(),
        }
    }
}
