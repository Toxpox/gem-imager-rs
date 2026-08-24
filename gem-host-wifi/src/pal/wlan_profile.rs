use crate::model::SecurityKind;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct WlanProfile {
    pub ssid: ProfileSsid,
    pub security: SecurityKind,
    pub credential: ProfileCredential,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ProfileSsid {
    Name(String),
    Bytes(Vec<u8>),
    Missing,
}

impl ProfileSsid {
    pub fn matches(&self, ssid: &str) -> bool {
        match self {
            Self::Name(name) => name == ssid,
            Self::Bytes(bytes) => bytes.as_slice() == ssid.as_bytes(),
            Self::Missing => false,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ProfileCredential {
    Plaintext(String),
    Encrypted,
    None,
    Unusable,
}

fn classify_authentication(auth: &str) -> SecurityKind {
    let auth = auth.trim().to_ascii_uppercase();
    match auth.as_str() {
        "WPAPSK" | "WPA2PSK" | "WPA3SAE" | "WPA3PSK" => SecurityKind::Personal,
        "WPA" | "WPA2" | "WPA3" | "WPA3ENTERPRISE" | "WPA3ENTERPRISE192" => {
            SecurityKind::Enterprise
        }
        "OWE" => SecurityKind::Open,
        "OPEN" => SecurityKind::Open,
        "SHARED" => SecurityKind::UnsupportedSecurity,
        _ => SecurityKind::Unknown,
    }
}

fn find_text<'a>(node: roxmltree::Node<'a, '_>, local_name: &str) -> Option<&'a str> {
    node.descendants()
        .find(|n| n.is_element() && n.tag_name().name() == local_name)
        .and_then(|n| n.text())
        .map(str::trim)
}

fn find_element<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.descendants()
        .find(|n| n.is_element() && n.tag_name().name() == local_name)
}

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

pub(crate) fn parse(xml: &str) -> Option<WlanProfile> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let root = doc.root_element();
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

fn parse_ssid(root: roxmltree::Node<'_, '_>) -> ProfileSsid {
    let Some(config) = find_element(root, "SSIDConfig") else {
        return ProfileSsid::Missing;
    };
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

fn parse_credential(root: roxmltree::Node<'_, '_>) -> ProfileCredential {
    let Some(shared_key) = find_element(root, "sharedKey") else {
        return ProfileCredential::None;
    };

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
        let xml = profile_xml(
            "<SSID><name>HomeNet</name></SSID>",
            r#"<authEncryption><authentication>WPA2PSK</authentication></authEncryption>
               <sharedKey><keyType>networkKey</keyType>
                          <protected>false</protected>
                          <keyMaterial>not-a-psk-value</keyMaterial></sharedKey>"#,
        );
        assert_eq!(
            parse(&xml).expect("parses").credential,
            ProfileCredential::Unusable
        );
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
        let xml = profile_xml(
            "<SSID><hex>486F6D654E6574</hex></SSID>",
            r#"<authEncryption><authentication>WPA2PSK</authentication></authEncryption>"#,
        );
        let parsed = parse(&xml).expect("parses");
        assert_eq!(parsed.ssid, ProfileSsid::Bytes(b"HomeNet".to_vec()));
        assert!(parsed.ssid.matches("HomeNet"));
        assert!(!parsed.ssid.matches("HomeNet2"));

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
