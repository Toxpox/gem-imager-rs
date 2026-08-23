//! The shipped udev rules are a packaging artifact, so nothing in the compiler checks them against
//! the IDs the DFU transport actually opens. `Msg::DfuPermissionBody` tells the user that
//! installing these rules will fix an EACCES on the board, so a drift between the two turns that
//! message into a dead end: the user installs the file and hits the same error.

use gem_config::t3::canonical::{T3_DFU_PRODUCT_ID, T3_DFU_VENDOR_ID};

const RULES: &str = include_str!("../assets/packages/linux/udev/10-t3gemstone.rules");

#[test]
fn shipped_rules_grant_access_to_the_dfu_device_the_flasher_opens() {
    let expected = format!(
        "ATTR{{idVendor}}==\"{T3_DFU_VENDOR_ID:04x}\", ATTR{{idProduct}}==\"{T3_DFU_PRODUCT_ID:04x}\""
    );

    assert!(
        RULES.contains(&expected),
        "no udev rule matches the DFU device {T3_DFU_VENDOR_ID:04x}:{T3_DFU_PRODUCT_ID:04x} \
         that gem-flasher-dfu opens; DfuPermissionBody would send users to a file that \
         cannot fix their error.\n--- rules ---\n{RULES}"
    );
}

#[test]
fn the_dfu_rule_actually_grants_the_invoking_user_access() {
    // MODE alone leaves the node owned by root:root. `uaccess` is what hands the device to the
    // user on the active seat, which is the case the error message is about.
    let dfu_line = RULES
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .find(|l| l.contains(&format!("{T3_DFU_PRODUCT_ID:04x}")))
        .expect("DFU rule is present");

    assert!(
        dfu_line.contains("TAG+=\"uaccess\""),
        "the DFU rule does not tag the device with uaccess, so it stays root-owned: {dfu_line}"
    );
}
