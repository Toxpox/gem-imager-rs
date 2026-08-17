//! Parsing of the Windows Native Wi-Fi profile XML.
//!
//! `WlanGetProfile` returns a `WLANProfile` document describing one saved network. Two things are
//! read out of it: the SSID (to match the profile against the connected network, §7.2) and, when
//! the plaintext flag was granted, the credential itself (§7.3).
//!
//! This module holds no Windows types, so the whole fixture matrix runs in the normal test suite on
//! any host. Element names are matched by *local* name and the namespace URI is not pinned, because
//! Microsoft revises the schema (v1, v2, v3) and some tools emit no namespace at all; the parser is
//! still namespace-aware, so prefixed documents resolve correctly rather than being string-matched
//! (§7.5, which also forbids regex extraction).

use crate::model::SecurityKind;

/// What a saved Windows profile says about one network.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct WlanProfile {
    pub ssid: ProfileSsid,
    pub security: SecurityKind,
    pub credential: ProfileCredential,
}

/// How a profile identifies its network: Windows writes either a literal name or a hex encoding of
/// the SSID bytes, and both have to be matchable against a detected SSID (§7.2).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ProfileSsid {
    Name(String),
    /// The raw SSID bytes, decoded from the `<hex>` form.
    Bytes(Vec<u8>),
    Missing,
}

impl ProfileSsid {
    /// Whether this profile is for `ssid`, comparing bytes so the hex and name forms agree.
    /// Matching is exact: a near match is not a match (§7.2).
    pub fn matches(&self, ssid: &str) -> bool {
        match self {
            Self::Name(name) => name == ssid,
            Self::Bytes(bytes) => bytes.as_slice() == ssid.as_bytes(),
            Self::Missing => false,
        }
    }
}

/// The credential state of a profile.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ProfileCredential {
    /// A plaintext key was present and is shaped like something the image can use.
    Plaintext(String),
    /// A key is present but still encrypted, because the plaintext flag was not granted (§7.4).
    Encrypted,
    /// The profile has no `sharedKey` at all: an open network, or an enterprise profile whose
    /// credentials live elsewhere.
    None,
    /// A key was present but cannot be used: an unusable length, or a hex key type whose material
    /// is not hexadecimal.
    Unusable,
}

/// The `<security>` fields that decide whether one portable passphrase can exist (§7.3).
///
/// Matched case-insensitively: Windows writes `WPA2PSK`, but other tools and older Windows versions
/// differ in case.
fn classify_authentication(auth: &str) -> SecurityKind {
    let auth = auth.trim().to_ascii_uppercase();
    match auth.as_str() {
        "WPAPSK" | "WPA2PSK" | "WPA3SAE" | "WPA3PSK" => SecurityKind::Personal,
        "WPA" | "WPA2" | "WPA3" | "WPA3ENTERPRISE" | "WPA3ENTERPRISE192" => SecurityKind::Enterprise,
        // OWE gives encryption without any passphrase to carry.
        "OWE" => SecurityKind::Open,
        "OPEN" => SecurityKind::Open,
        // Static WEP: secured, but not with something the image can take.
        "SHARED" => SecurityKind::UnsupportedSecurity,
        _ => SecurityKind::Unknown,
    }
}

/// Find the first descendant with this local name, ignoring the namespace URI.
fn find_text<'a>(node: roxmltree::Node<'a, '_>, local_name: &str) -> Option<&'a str> {
    node.descendants()
        .find(|n| n.is_element() && n.tag_name().name() == local_name)
        .and_then(|n| n.text())
        .map(str::trim)
}

/// Find the first descendant element with this local name.
fn find_element<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.descendants()
        .find(|n| n.is_element() && n.tag_name().name() == local_name)
}

/// Decode the `<hex>` form of an SSID. Returns `None` for anything that is not an even-length run
/// of hex digits, so a malformed profile is skipped rather than matched against a corrupted name.
fn decode_hex_ssid(hex: &str) -> Option<Vec<u8>> {
    let hex = hex.trim();
    if hex.is_empty() || !hex.len().is_multiple_of(2) || !hex.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

/// Parse a `WLANProfile` document.
///
/// `None` means the document is not well-formed XML or is not a WLAN profile at all. A document
/// that parses but is missing pieces is described through the returned value instead, because a
/// profile can legitimately lack a `sharedKey` while still being the right profile.
pub(crate) fn parse(xml: &str) -> Option<WlanProfile> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let root = doc.root_element();
    // Guard against being handed some other document entirely.
    if root.tag_name().name() != "WLANProfile" {
        return None;
    }

    let ssid = parse_ssid(root);
    let security = find_text(root, "authentication")
        .map(classify_authentication)
        .unwrap_or(SecurityKind::Unknown);
    let credential = parse_credential(root);

    Some(WlanProfile {
        ssid,
        security,
        credential,
    })
}

/// Read the SSID from `<SSIDConfig>`, preferring the literal name and falling back to the hex form.
fn parse_ssid(root: roxmltree::Node<'_, '_>) -> ProfileSsid {
    let Some(config) = find_element(root, "SSIDConfig") else {
        return ProfileSsid::Missing;
    };
    // Look inside <SSID> when it is there, so the profile's own <name> element (a sibling of
    // SSIDConfig, holding the *profile* name) is never mistaken for the SSID.
    let scope = find_element(config, "SSID").unwrap_or(config);

    if let Some(name) = find_text(scope, "name")
        && !name.is_empty()
    {
        return ProfileSsid::Name(name.to_owned());
    }
    if let Some(bytes) = find_text(scope, "hex").and_then(decode_hex_ssid) {
        return ProfileSsid::Bytes(bytes);
    }
    ProfileSsid::Missing
}

/// Read `<sharedKey>`, honouring `<protected>` and `<keyType>` (§7.3).
fn parse_credential(root: roxmltree::Node<'_, '_>) -> ProfileCredential {
    let Some(shared_key) = find_element(root, "sharedKey") else {
        return ProfileCredential::None;
    };

    // `protected` true means the material is DPAPI-encrypted: the plaintext flag was not granted,
    // or the caller lacked the rights for it. Trying to decrypt it is forbidden (§7.7).
    let protected = find_text(shared_key, "protected")
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let Some(material) = find_text(shared_key, "keyMaterial") else {
        return ProfileCredential::None;
    };
    if material.is_empty() {
        return ProfileCredential::None;
    }
    if protected {
        return ProfileCredential::Encrypted;
    }

    // `networkKey` is a 64-hex-digit PSK; `passPhrase` is the 8..=63 byte form. Both are checked
    // against the T3 serializer's contract, so an unusable value is reported rather than pushed
    // into the form (§3.1).
    let key_type = find_text(shared_key, "keyType").unwrap_or_default();
    if key_type.eq_ignore_ascii_case("networkKey")
        && !(material.len() == 64 && material.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return ProfileCredential::Unusable;
    }
    if !crate::is_usable_wifi_credential(material) {
        return ProfileCredential::Unusable;
    }

    ProfileCredential::Plaintext(material.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NS: &str = r#"xmlns="http://www.microsoft.com/networking/WLAN/profile/v1""#;

    fn profile_xml(ssid_config: &str, security: &str) -> String {
        format!(
            r#"<?xml version="1.0"?>
<WLANProfile {NS}>
  <name>SomeProfileName</name>
  <SSIDConfig>{ssid_config}</SSIDConfig>
  <connectionType>ESS</connectionType>
  <MSM><security>{security}</security></MSM>
</WLANProfile>"#
        )
    }

    fn wpa2_personal(key: &str) -> String {
        profile_xml(
            "<SSID><name>HomeNet</name></SSID>",
            &format!(
                r#"<authEncryption>
                     <authentication>WPA2PSK</authentication>
                     <encryption>AES</encryption>
                     <useOneX>false</useOneX>
                   </authEncryption>
                   <sharedKey>
                     <keyType>passPhrase</keyType>
                     <protected>false</protected>
                     <keyMaterial>{key}</keyMaterial>
                   </sharedKey>"#
            ),
        )
    }

    #[test]
    fn a_wpa2_personal_passphrase_is_read() {
        let parsed = parse(&wpa2_personal("hunter2-pass")).expect("parses");
        assert_eq!(parsed.ssid, ProfileSsid::Name("HomeNet".to_owned()));
        assert_eq!(parsed.security, SecurityKind::Personal);
        assert_eq!(
            parsed.credential,
            ProfileCredential::Plaintext("hunter2-pass".to_owned())
        );
    }

    #[test]
    fn a_64_hex_psk_is_read_as_a_network_key() {
        let psk = "0DC0D6EB90555ED6419756B9A15EC3E3209B63DF707DD508D14581F8982721AF";
        let xml = profile_xml(
            "<SSID><name>HomeNet</name></SSID>",
            &format!(
                r#"<authEncryption><authentication>WPA2PSK</authentication>
                                   <encryption>AES</encryption></authEncryption>
                   <sharedKey><keyType>networkKey</keyType>
                              <protected>false</protected>
                              <keyMaterial>{psk}</keyMaterial></sharedKey>"#
            ),
        );
        let parsed = parse(&xml).expect("parses");
        assert_eq!(
            parsed.credential,
            ProfileCredential::Plaintext(psk.to_owned())
        );
    }

    #[test]
    fn a_networkkey_that_is_not_hex_is_unusable() {
        // A networkKey must be a real PSK; a passphrase-shaped value under that key type is a
        // malformed profile, not a credential to hand over.
        let xml = profile_xml(
            "<SSID><name>HomeNet</name></SSID>",
            r#"<authEncryption><authentication>WPA2PSK</authentication></authEncryption>
               <sharedKey><keyType>networkKey</keyType>
                          <protected>false</protected>
                          <keyMaterial>not-a-psk-value</keyMaterial></sharedKey>"#,
        );
        assert_eq!(parse(&xml).expect("parses").credential, ProfileCredential::Unusable);
    }

    #[test]
    fn wpa3_sae_is_personal() {
        let xml = profile_xml(
            "<SSID><name>HomeNet</name></SSID>",
            r#"<authEncryption><authentication>WPA3SAE</authentication>
                               <encryption>AES</encryption></authEncryption>
               <sharedKey><keyType>passPhrase</keyType>
                          <protected>false</protected>
                          <keyMaterial>sae-passphrase</keyMaterial></sharedKey>"#,
        );
        let parsed = parse(&xml).expect("parses");
        assert_eq!(parsed.security, SecurityKind::Personal);
        assert!(matches!(parsed.credential, ProfileCredential::Plaintext(_)));
    }

    #[test]
    fn a_protected_key_is_reported_as_encrypted_not_decrypted() {
        let xml = profile_xml(
            "<SSID><name>HomeNet</name></SSID>",
            r#"<authEncryption><authentication>WPA2PSK</authentication></authEncryption>
               <sharedKey><keyType>passPhrase</keyType>
                          <protected>true</protected>
                          <keyMaterial>01000000D08C9DDF0115D1118C7A00C04FC297EB</keyMaterial></sharedKey>"#,
        );
        assert_eq!(
            parse(&xml).expect("parses").credential,
            ProfileCredential::Encrypted
        );
    }

    #[test]
    fn enterprise_open_and_owe_profiles_carry_no_portable_key() {
        let enterprise = profile_xml(
            "<SSID><name>CorpNet</name></SSID>",
            r#"<authEncryption><authentication>WPA2</authentication>
                               <encryption>AES</encryption>
                               <useOneX>true</useOneX></authEncryption>"#,
        );
        let parsed = parse(&enterprise).expect("parses");
        assert_eq!(parsed.security, SecurityKind::Enterprise);
        assert_eq!(parsed.credential, ProfileCredential::None);

        let open = profile_xml(
            "<SSID><name>CafeWifi</name></SSID>",
            r#"<authEncryption><authentication>open</authentication>
                               <encryption>none</encryption></authEncryption>"#,
        );
        let parsed = parse(&open).expect("parses");
        assert_eq!(parsed.security, SecurityKind::Open);
        assert_eq!(parsed.credential, ProfileCredential::None);

        let owe = profile_xml(
            "<SSID><name>OpenSecure</name></SSID>",
            r#"<authEncryption><authentication>OWE</authentication>
                               <encryption>AES</encryption></authEncryption>"#,
        );
        assert_eq!(parse(&owe).expect("parses").security, SecurityKind::Open);
    }

    #[test]
    fn static_wep_is_unsupported_rather_than_open() {
        let wep = profile_xml(
            "<SSID><name>OldNet</name></SSID>",
            r#"<authEncryption><authentication>shared</authentication>
                               <encryption>WEP</encryption></authEncryption>"#,
        );
        assert_eq!(
            parse(&wep).expect("parses").security,
            SecurityKind::UnsupportedSecurity
        );
    }

    #[test]
    fn the_hex_ssid_form_is_decoded_and_matches_the_same_network() {
        // "HomeNet" in hex.
        let xml = profile_xml(
            "<SSID><hex>486F6D654E6574</hex></SSID>",
            r#"<authEncryption><authentication>WPA2PSK</authentication></authEncryption>"#,
        );
        let parsed = parse(&xml).expect("parses");
        assert_eq!(parsed.ssid, ProfileSsid::Bytes(b"HomeNet".to_vec()));
        assert!(parsed.ssid.matches("HomeNet"));
        assert!(!parsed.ssid.matches("HomeNet2"));

        // Odd length and non-hex content are rejected rather than partially decoded.
        for bad in ["486F6D654E657", "zzzz", ""] {
            let xml = profile_xml(
                &format!("<SSID><hex>{bad}</hex></SSID>"),
                r#"<authEncryption><authentication>WPA2PSK</authentication></authEncryption>"#,
            );
            assert_eq!(parse(&xml).expect("parses").ssid, ProfileSsid::Missing);
        }
    }

    #[test]
    fn ssid_matching_is_exact_and_never_uses_the_profile_name() {
        // The <name> directly under WLANProfile is the *profile* name, which the plan says must not
        // be assumed equal to the SSID (§7.2). Only the one inside SSIDConfig/SSID counts.
        let xml = profile_xml(
            "<SSID><name>ActualSsid</name></SSID>",
            r#"<authEncryption><authentication>WPA2PSK</authentication></authEncryption>"#,
        );
        let parsed = parse(&xml).expect("parses");
        assert!(parsed.ssid.matches("ActualSsid"));
        assert!(!parsed.ssid.matches("SomeProfileName"));
    }

    #[test]
    fn a_prefixed_namespace_is_resolved_not_string_matched() {
        // The same document written with an explicit prefix must parse identically.
        let xml = r#"<?xml version="1.0"?>
<p:WLANProfile xmlns:p="http://www.microsoft.com/networking/WLAN/profile/v1">
  <p:name>Profile</p:name>
  <p:SSIDConfig><p:SSID><p:name>HomeNet</p:name></p:SSID></p:SSIDConfig>
  <p:MSM><p:security>
    <p:authEncryption><p:authentication>WPA2PSK</p:authentication></p:authEncryption>
    <p:sharedKey><p:keyType>passPhrase</p:keyType>
                 <p:protected>false</p:protected>
                 <p:keyMaterial>hunter2-pass</p:keyMaterial></p:sharedKey>
  </p:security></p:MSM>
</p:WLANProfile>"#;
        let parsed = parse(xml).expect("parses");
        assert_eq!(parsed.ssid, ProfileSsid::Name("HomeNet".to_owned()));
        assert_eq!(parsed.security, SecurityKind::Personal);
        assert_eq!(
            parsed.credential,
            ProfileCredential::Plaintext("hunter2-pass".to_owned())
        );
    }

    #[test]
    fn a_later_schema_revision_still_parses() {
        // Microsoft revises the schema URI; pinning to v1 would break on a v3 document.
        let xml = wpa2_personal("hunter2-pass").replace("/v1", "/v3");
        let parsed = parse(&xml).expect("parses");
        assert_eq!(parsed.security, SecurityKind::Personal);
        assert!(matches!(parsed.credential, ProfileCredential::Plaintext(_)));
    }

    #[test]
    fn a_group_policy_profile_with_extra_elements_still_parses() {
        let xml = r#"<?xml version="1.0"?>
<WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1">
  <name>GP Profile</name>
  <SSIDConfig><SSID><name>CorpNet</name></SSID><nonBroadcast>true</nonBroadcast></SSIDConfig>
  <connectionType>ESS</connectionType>
  <connectionMode>auto</connectionMode>
  <autoSwitch>false</autoSwitch>
  <MSM>
    <security>
      <authEncryption><authentication>WPA2PSK</authentication>
                      <encryption>AES</encryption>
                      <useOneX>false</useOneX>
                      <FIPSMode xmlns="http://www.microsoft.com/networking/WLAN/profile/v2">false</FIPSMode>
      </authEncryption>
      <sharedKey><keyType>passPhrase</keyType>
                 <protected>false</protected>
                 <keyMaterial>corp-passphrase</keyMaterial></sharedKey>
    </security>
  </MSM>
</WLANProfile>"#;
        let parsed = parse(xml).expect("parses");
        assert_eq!(parsed.ssid, ProfileSsid::Name("CorpNet".to_owned()));
        assert_eq!(
            parsed.credential,
            ProfileCredential::Plaintext("corp-passphrase".to_owned())
        );
    }

    #[test]
    fn malformed_and_foreign_documents_are_rejected() {
        assert!(parse("not xml at all <<<").is_none());
        assert!(parse("<WLANProfile><unclosed></WLANProfile>").is_none());
        assert!(parse("").is_none());
        // Well-formed XML that is not a profile must not be mined for fields.
        assert!(parse(r#"<?xml version="1.0"?><SomethingElse><keyMaterial>x</keyMaterial></SomethingElse>"#).is_none());
    }

    #[test]
    fn a_key_of_an_unusable_length_is_not_handed_over() {
        for bad in ["short", &"a".repeat(70)] {
            let parsed = parse(&wpa2_personal(bad)).expect("parses");
            assert_eq!(
                parsed.credential,
                ProfileCredential::Unusable,
                "a {}-byte key should not be handed over",
                bad.len()
            );
        }
        // An empty keyMaterial is "no key", not an unusable one.
        let parsed = parse(&wpa2_personal("")).expect("parses");
        assert_eq!(parsed.credential, ProfileCredential::None);
    }

    #[test]
    fn a_profile_without_an_ssid_config_matches_nothing() {
        let xml = profile_xml(
            "",
            r#"<authEncryption><authentication>WPA2PSK</authentication></authEncryption>"#,
        );
        let parsed = parse(&xml).expect("parses");
        assert_eq!(parsed.ssid, ProfileSsid::Missing);
        assert!(!parsed.ssid.matches("HomeNet"));
        assert!(!parsed.ssid.matches(""));
    }
}
