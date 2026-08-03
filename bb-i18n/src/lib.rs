//! Turkish and English user-facing strings for the T3 Gemstone Imager.
//!
//! # Why this is a compile-time table and not a resource loader
//!
//! The rest of this codebase refuses silent degradation: a catalog that parses to zero images is
//! not a success, an archive whose hash does not match is not "probably fine". A localisation
//! layer that falls back to the key name — or to English — when a translation is missing is the
//! same failure in a different costume, and it fails in exactly the place where it hurts most:
//! the destructive-action confirmation that the user is reading in their own language.
//!
//! So [`Msg`] and the lookup table are generated together by one macro. Adding a variant without
//! a Turkish string does not compile. There is no runtime path that can produce a missing string,
//! which is also why there is no `Result` in this API.
//!
//! # Strings with values in them
//!
//! Anything that interpolates lives in [`fmt`] as a function taking typed arguments, rather than
//! as a template the caller fills in positionally. That keeps argument order from drifting
//! between languages — Turkish word order routinely differs from English — and makes each
//! message individually snapshot-testable in both languages.
//!
//! # Scope
//!
//! Keys for flows that are not built yet (DFU, WinUSB/Zadig, udev permissions) are present.
//! `instruction.md` §11.2 asks for the resources to exist *before* the UI text grows, so Faz 8
//! finds them ready instead of adding English literals it would have to revisit.

#![forbid(unsafe_code)]

/// A language the interface can be displayed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Lang {
    /// English. The default when the system locale is anything other than Turkish.
    #[default]
    En,
    /// Turkish.
    Tr,
}

impl Lang {
    /// Every supported language, in the order a picker should show them.
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Tr];

    /// The ISO 639-1 code, as stored in the persisted GUI configuration.
    pub const fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Tr => "tr",
        }
    }

    /// Parse a stored or system code. Returns `None` for anything unsupported, so the caller
    /// decides what to do rather than being handed a silent default.
    pub fn from_code(code: &str) -> Option<Self> {
        // Accepts "tr", "tr_TR", "tr-TR.UTF-8" and the same shapes for English.
        let primary = code
            .split(['_', '-', '.'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();

        match primary.as_str() {
            "en" => Some(Lang::En),
            "tr" => Some(Lang::Tr),
            _ => None,
        }
    }

    /// The language's own name, for the language picker. Never translated: a user looking for
    /// their language finds it written the way they write it.
    pub const fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Tr => "Türkçe",
        }
    }
}

impl std::fmt::Display for Lang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.native_name())
    }
}

/// Generates [`Msg`] and the lookup table from one list, so the two cannot drift apart.
///
/// A variant declared without a `tr:` arm is a syntax error; a variant added to `Msg` by hand
/// would leave the `match` in `text` non-exhaustive. Either way the build stops.
macro_rules! catalog {
    ($( $(#[$meta:meta])* $key:ident { en: $en:literal, tr: $tr:literal } ),* $(,)?) => {
        /// A user-facing string with no interpolated values.
        ///
        /// Messages that carry values are functions in [`fmt`] instead.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Msg {
            $( $(#[$meta])* $key ),*
        }

        impl Msg {
            /// Every message, for the test that asserts no string was left untranslated.
            pub const ALL: &'static [Msg] = &[ $( Msg::$key ),* ];
        }

        impl Lang {
            /// The message, in this language.
            pub const fn text(self, msg: Msg) -> &'static str {
                match (self, msg) {
                    $(
                        (Lang::En, Msg::$key) => $en,
                        (Lang::Tr, Msg::$key) => $tr,
                    )*
                }
            }
        }
    };
}

catalog! {
    // ---- Navigation and shared controls -------------------------------------------------
    Back { en: "BACK", tr: "GERİ" },
    Next { en: "NEXT", tr: "İLERİ" },
    Reset { en: "RESET", tr: "SIFIRLA" },
    Cancel { en: "Cancel", tr: "İptal" },
    Search { en: "SEARCH", tr: "ARA" },
    Documentation { en: "DOCUMENTATION", tr: "BELGELER" },
    Support { en: "SUPPORT", tr: "DESTEK" },
    Language { en: "Language", tr: "Dil" },

    // ---- Board, image and destination selection -----------------------------------------
    SelectBoardPrompt { en: "Please Select a Board", tr: "Lütfen bir kart seçin" },
    SelectOsPrompt { en: "Please Select an OS", tr: "Lütfen bir işletim sistemi seçin" },
    SelectDestinationPrompt { en: "Please Select a Destination", tr: "Lütfen bir hedef seçin" },
    ShowAllDestinations { en: "Show all destinations", tr: "Tüm hedefleri göster" },
    SpecialInstructions { en: "Special instructions", tr: "Özel talimatlar" },
    InitFormat { en: "Init Format", tr: "Başlangıç biçimi" },
    InvalidImage { en: "Invalid image", tr: "Geçersiz imaj" },
    SelectLocalImage { en: "Select Local Image", tr: "Yerel imaj seç" },
    FormatSdCard { en: "Format SD Card", tr: "SD kartı biçimlendir" },
    FormatSdCardDesc {
        en: "Format a SD Card to FAT32 for reuse.",
        tr: "SD kartı yeniden kullanmak için FAT32 olarak biçimlendirir."
    },
    FileDoesNotExist { en: "File does not exist", tr: "Dosya bulunamadı" },

    // ---- Review -------------------------------------------------------------------------
    ReviewTitle { en: "Write Image", tr: "İmajı yaz" },
    ReviewSubtitle {
        en: "Review your choices before flashing",
        tr: "Yazmadan önce seçimlerinizi gözden geçirin"
    },
    Summary { en: "Summary", tr: "Özet" },
    Device { en: "Device", tr: "Kart" },
    OperatingSystem { en: "Operating System", tr: "İşletim sistemi" },
    Storage { en: "Storage", tr: "Depolama" },
    ModificationsToApply { en: "Modifications to apply", tr: "Uygulanacak değişiklikler" },
    NoCustomization { en: "No customization", tr: "Özelleştirme yok" },
    DownloadAction { en: "DOWNLOAD", tr: "İNDİR" },
    WriteAction { en: "WRITE", tr: "YAZ" },
    DownloadSize { en: "Download Size", tr: "İndirme boyutu" },

    // ---- Data-loss confirmation (`instruction.md` §11.2) --------------------------------
    EraseConfirmTitle { en: "Erase all data?", tr: "Tüm veriler silinsin mi?" },
    EraseConfirmAccept { en: "Erase and write", tr: "Sil ve yaz" },
    EraseConfirmReject { en: "Keep my data", tr: "Vazgeç" },

    // ---- Progress -----------------------------------------------------------------------
    Preparing { en: "Preparing ...", tr: "Hazırlanıyor ..." },
    Downloading { en: "Downloading ...", tr: "İndiriliyor ..." },
    FlashingImage { en: "Flashing Image ...", tr: "İmaj yazılıyor ..." },
    VerifyingWrittenData { en: "Verifying written data ...", tr: "Yazılan veri doğrulanıyor ..." },
    Customizing { en: "Customizing ...", tr: "Özelleştiriliyor ..." },
    TimeRemaining { en: "Time Remaining", tr: "Kalan süre" },
    // DFU phases. The eMMC flow has no single "flashing" step, and naming the phases separately is
    // what lets the user tell a board that is busy from a board that has stopped responding.
    PreparingImage { en: "Preparing image ...", tr: "İmaj hazırlanıyor ..." },
    ResolvingBootArtifacts {
        en: "Verifying boot files ...",
        tr: "Boot dosyaları doğrulanıyor ..."
    },
    ChecksummingImage {
        en: "Checksumming image ...",
        tr: "İmaj sağlaması hesaplanıyor ..."
    },
    WaitingForBoard {
        en: "Waiting for the board to reconnect ...",
        tr: "Kartın yeniden bağlanması bekleniyor ..."
    },
    WritingBootloader { en: "Writing bootloader ...", tr: "Önyükleyici yazılıyor ..." },
    WritingToEmmc { en: "Writing to eMMC ...", tr: "eMMC'ye yazılıyor ..." },
    FinalizingWrite {
        en: "Finishing on the board, do not disconnect ...",
        tr: "Kart üzerinde tamamlanıyor, bağlantıyı kesmeyin ..."
    },

    // ---- Terminal states ----------------------------------------------------------------
    Failed { en: "Failed", tr: "Başarısız" },
    Cancelled { en: "Cancelled", tr: "İptal edildi" },
    CancelledByUser {
        en: "Flashing Cancelled by the user",
        tr: "Yazma işlemi kullanıcı tarafından iptal edildi"
    },
    FlashSuccess { en: "Successfully Flashed Image", tr: "İmaj başarıyla yazıldı" },
    DownloadSuccess { en: "Successfully Downloaded Image", tr: "İmaj başarıyla indirildi" },
    FlashAnother { en: "Flash Another", tr: "Yeni imaj yaz" },
    FlashNew { en: "Flash New", tr: "Yeni yazma" },
    Retry { en: "Retry", tr: "Tekrar dene" },
    Restart { en: "Restart", tr: "Yeniden başla" },
    Logs { en: "Logs", tr: "Günlükler" },

    // ---- Integrity failures (`instruction.md` §11.2) ------------------------------------
    IntegrityFailedTitle { en: "The image is damaged", tr: "İmaj bozuk" },
    IntegrityFailedBody {
        en: "The downloaded image does not match the checksum published for it. Nothing was written to your card. Check your connection and download it again.",
        tr: "İndirilen imaj, kendisi için yayımlanan sağlama değeriyle eşleşmiyor. Kartınıza hiçbir şey yazılmadı. Bağlantınızı denetleyip yeniden indirin."
    },
    ReadBackFailedTitle { en: "The card did not keep what was written", tr: "Kart yazılanı korumadı" },
    ReadBackFailedBody {
        en: "The data read back from the card differs from the image. The card may be failing or counterfeit. Do not boot from it — try another card.",
        tr: "Karttan geri okunan veri imajdan farklı. Kart arızalı veya sahte olabilir. Bu karttan açılış yapmayın — başka bir kart deneyin."
    },

    // ---- Destination safety (`instruction.md` §11.2) ------------------------------------
    SystemDiskRefusedTitle { en: "This is your system disk", tr: "Bu, sisteminizin diski" },
    SystemDiskRefusedBody {
        en: "Writing here would destroy the operating system you are running. This destination is not available.",
        tr: "Buraya yazmak, üzerinde çalıştığınız işletim sistemini yok eder. Bu hedef kullanılamaz."
    },
    DestinationTooSmallTitle { en: "The card is too small", tr: "Kart çok küçük" },
    DestinationTooSmallBody {
        en: "The selected image does not fit on this card. Use a larger card and try again.",
        tr: "Seçilen imaj bu karta sığmıyor. Daha büyük bir kart kullanıp yeniden deneyin."
    },
    DestinationRemovedTitle { en: "The card was removed", tr: "Kart çıkarıldı" },
    DestinationRemovedBody {
        en: "The destination disappeared while writing. The card is now in an unknown state — write it again from the beginning before using it.",
        tr: "Hedef, yazma sırasında kayboldu. Kart artık bilinmeyen bir durumda — kullanmadan önce baştan yeniden yazın."
    },

    // ---- Catalog freshness (`instruction.md` §11.2) -------------------------------------
    CatalogOfflineTitle { en: "Showing a saved image list", tr: "Kayıtlı imaj listesi gösteriliyor" },
    CatalogOfflineBody {
        en: "The image list could not be fetched, so the last one that was known good is being shown. It may be out of date.",
        tr: "İmaj listesi alınamadı; bu yüzden bilinen son geçerli liste gösteriliyor. Güncel olmayabilir."
    },
    CatalogRefresh { en: "Refresh", tr: "Yenile" },
    CatalogEmptyTitle { en: "No images could be read", tr: "Hiçbir imaj okunamadı" },
    CatalogEmptyBody {
        en: "The image list was reached but nothing in it could be used. This is a problem with the list, not with your setup — please report it.",
        tr: "İmaj listesine ulaşıldı ancak içindeki hiçbir kayıt kullanılamadı. Bu, kurulumunuzla değil listeyle ilgili bir sorun — lütfen bildirin."
    },

    // ---- Linux permissions (`instruction.md` §11.2) -------------------------------------
    UdevPermissionTitle { en: "Permission denied for this disk", tr: "Bu disk için izin verilmedi" },
    UdevPermissionBody {
        en: "Your user is not allowed to write to the card directly. Install the udev rules that ship with this application, then unplug the card and plug it back in.",
        tr: "Kullanıcınızın karta doğrudan yazma izni yok. Bu uygulamayla gelen udev kurallarını kurun, ardından kartı çıkarıp yeniden takın."
    },

    // ---- DFU / eMMC, wired up in Faz 8 (`instruction.md` §11.2) -------------------------
    DfuSwitchToBootModeTitle { en: "Put the board in boot mode", tr: "Kartı boot moduna alın" },
    DfuSwitchToBootModeBody {
        en: "Power the board off, move the boot switch to the boot-mode position, then connect the USB cable. The board should appear as a DFU device.",
        tr: "Kartın gücünü kesin, boot anahtarını boot modu konumuna alın, ardından USB kablosunu takın. Kart bir DFU aygıtı olarak görünmelidir."
    },
    DfuDoNotDisconnectTitle { en: "Do not disconnect the board", tr: "Kartın bağlantısını kesmeyin" },
    DfuDoNotDisconnectBody {
        en: "Writing to eMMC takes several minutes and the board reconnects itself between stages. Removing power or the cable during this leaves the eMMC unbootable.",
        tr: "eMMC'ye yazma birkaç dakika sürer ve kart aşamalar arasında kendini yeniden bağlar. Bu sırada gücü veya kabloyu kesmek eMMC'yi açılamaz durumda bırakır."
    },
    DfuSwitchBackTitle { en: "Move the switch back to eMMC", tr: "Anahtarı eMMC konumuna geri alın" },
    DfuSwitchBackBody {
        en: "Writing is finished. Power the board off, move the boot switch back to the eMMC position, then power it on to boot from eMMC.",
        tr: "Yazma tamamlandı. Kartın gücünü kesin, boot anahtarını eMMC konumuna geri alın, ardından eMMC'den açılması için gücü verin."
    },
    DfuReconnectTimeoutTitle { en: "The board did not come back", tr: "Kart geri dönmedi" },
    DfuReconnectTimeoutBody {
        en: "The board was reset but did not reappear in time. Nothing further was written. Unplug it, plug it back in, and start again.",
        tr: "Kart sıfırlandı ancak zamanında yeniden görünmedi. Başka hiçbir şey yazılmadı. Kabloyu çıkarıp yeniden takın ve baştan başlayın."
    },
    DfuManifestFailedTitle { en: "Boot files could not be verified", tr: "Boot dosyaları doğrulanamadı" },
    DfuManifestFailedBody {
        en: "The boot files for this board could not be downloaded or did not match their published checksums. Writing was not started.",
        tr: "Bu kart için boot dosyaları indirilemedi veya yayımlanan sağlama değerleriyle eşleşmedi. Yazma işlemi başlatılmadı."
    },
    DfuPermissionTitle { en: "Permission denied for the board", tr: "Kart için izin verilmedi" },
    DfuPermissionBody {
        en: "Your user is not allowed to open this USB device. Install the udev rules that ship with this application, then unplug the board and plug it back in.",
        tr: "Kullanıcınızın bu USB aygıtını açma izni yok. Bu uygulamayla gelen udev kurallarını kurun, ardından kartı çıkarıp yeniden takın."
    },
    DfuNoDeviceTitle { en: "No board in boot mode was found", tr: "Boot modunda kart bulunamadı" },
    DfuNoDeviceBody {
        en: "The board is no longer on the USB port it was selected on. Nothing was written. Check the cable and the boot switch, then select the destination again.",
        tr: "Kart, seçildiği USB portunda artık görünmüyor. Hiçbir şey yazılmadı. Kabloyu ve boot anahtarını kontrol edip hedefi yeniden seçin."
    },
    DfuAmbiguousTitle { en: "More than one board is connected", tr: "Birden fazla kart bağlı" },
    DfuAmbiguousBody {
        en: "Several boards in boot mode are attached to this computer, so the right one cannot be chosen for you. Nothing was written. Disconnect all but the board you want to write to, then select it again.",
        tr: "Bu bilgisayara boot modunda birden fazla kart bağlı; doğru olanı sizin yerinize seçemeyiz. Hiçbir şey yazılmadı. Yazmak istediğiniz kart dışındakileri çıkarın ve hedefi yeniden seçin."
    },
    DfuTransferFailedTitle { en: "The transfer to the board failed", tr: "Karta aktarım başarısız oldu" },
    DfuTransferFailedBody {
        en: "The board stopped accepting data part way through. Its storage is now partly written and will not boot. Check the cable and the USB port, then run the write again from the start.",
        tr: "Kart, aktarımın ortasında veri almayı durdurdu. Kartın deposu artık kısmen yazılmış durumda ve açılmayacaktır. Kabloyu ve USB portunu kontrol edip yazmayı baştan çalıştırın."
    },
    DfuFinalizeFailedTitle { en: "The board did not confirm the write", tr: "Kart yazmayı onaylamadı" },
    DfuFinalizeFailedBody {
        en: "All data was sent, but the board never reported that it finished writing to eMMC. Treat the eMMC as incomplete: power the board off, connect it in boot mode again and repeat the write.",
        tr: "Tüm veri gönderildi ancak kart eMMC'ye yazmayı bitirdiğini hiç bildirmedi. eMMC'yi eksik kabul edin: kartın gücünü kesin, boot modunda yeniden bağlayın ve yazmayı tekrarlayın."
    },
    DfuDestinationSubtitle { en: "Onboard eMMC (DFU)", tr: "Karta gömülü eMMC (DFU)" },
    StagingSpaceTitle { en: "Not enough disk space", tr: "Yeterli disk alanı yok" },
    StagingSpaceBody {
        en: "Writing to eMMC first prepares a full copy of the image on this computer, and there is not enough free space for it. Free up space and try again; the board was not touched.",
        tr: "eMMC'ye yazma önce bu bilgisayarda imajın tam bir kopyasını hazırlar ve bunun için yeterli boş alan yok. Alan açıp yeniden deneyin; karta dokunulmadı."
    },
    WinusbDriverMissingTitle { en: "The USB driver is missing", tr: "USB sürücüsü eksik" },
    WinusbDriverMissingBody {
        en: "Windows has no WinUSB driver bound to the board, so it cannot be opened. Assign WinUSB to the device with Zadig, then connect it again.",
        tr: "Windows'ta karta bağlı bir WinUSB sürücüsü yok, bu yüzden aygıt açılamıyor. Zadig ile aygıta WinUSB atayın, ardından yeniden bağlayın."
    },

    // ---- Customization ------------------------------------------------------------------
    SetHostname { en: "Set Hostname", tr: "Makine adı belirle" },
    SetPassword { en: "Set Password", tr: "Parola belirle" },
    Password { en: "Password", tr: "Parola" },
    Username { en: "Username", tr: "Kullanıcı adı" },
    ConfigureWirelessLan { en: "Configure Wireless LAN", tr: "Kablosuz ağı yapılandır" },
    Ssid { en: "SSID", tr: "SSID" },
    Country { en: "Country", tr: "Ülke" },
    SetTimezone { en: "Set Timezone", tr: "Saat dilimi belirle" },
    Timezone { en: "Timezone", tr: "Saat dilimi" },
    SetKeymap { en: "Set Keymap", tr: "Klavye düzeni belirle" },
    Keymap { en: "Keymap", tr: "Klavye düzeni" },
    EnableVnc { en: "Enable VNC", tr: "VNC'yi etkinleştir" },
    ConfigureUsernamePassword {
        en: "Configure Username and Password",
        tr: "Kullanıcı adı ve parola yapılandır"
    },
    Hostname { en: "Hostname", tr: "Makine adı" },
    EnableUsbDhcp { en: "Enable USB DHCP", tr: "USB DHCP'yi etkinleştir" },
    SshAuthorizationKey { en: "SSH authorization public key", tr: "SSH yetkilendirme açık anahtarı" },
    WifiKeyHint {
        en: "The passphrase is converted to a network key before it is written, so it never reaches the card. A 64-digit hexadecimal key is accepted as-is.",
        tr: "Parola yazılmadan önce ağ anahtarına dönüştürülür; bu nedenle karta hiçbir zaman ulaşmaz. 64 haneli onaltılık anahtar olduğu gibi kabul edilir."
    },
    VncProtocolHint {
        en: "VNC passwords are limited to 8 characters by the protocol. A longer one is rejected rather than silently shortened.",
        tr: "VNC parolaları protokol gereği 8 karakterle sınırlıdır. Daha uzun parola sessizce kısaltılmak yerine reddedilir."
    },
    VncKnownIssueHint {
        en: "Known issue: the board's first-boot script does not remove the VNC password from the boot partition, so it stays readable on the card.",
        tr: "Bilinen sorun: kartın ilk açılış betiği VNC parolasını boot bölümünden silmez; parola kart üzerinde okunabilir kalır."
    },
    InvalidControlCharacter {
        en: "A field contains a control character. Remove line breaks and other control characters.",
        tr: "Bir alan denetim karakteri içeriyor. Satır sonlarını ve diğer denetim karakterlerini kaldırın."
    },
    InvalidHostnameError {
        en: "Enter a valid hostname using letters, digits and hyphens.",
        tr: "Harf, rakam ve kısa çizgi kullanarak geçerli bir makine adı girin."
    },
    InvalidWifiCountryError {
        en: "Enter the Wi-Fi country as two upper-case letters, for example TR.",
        tr: "Wi-Fi ülkesini TR gibi iki büyük harfle girin."
    },
    InvalidSsidError {
        en: "Enter a Wi-Fi name between 1 and 32 bytes.",
        tr: "1 ile 32 bayt arasında bir Wi-Fi adı girin."
    },
    UnknownTimezoneError {
        en: "Select a timezone offered by the application.",
        tr: "Uygulamanın sunduğu saat dilimlerinden birini seçin."
    },
    UnknownKeymapError {
        en: "Select a keyboard layout offered by the application.",
        tr: "Uygulamanın sunduğu klavye düzenlerinden birini seçin."
    },
    InvalidWifiPasswordError {
        en: "Use an 8–63 character Wi-Fi passphrase or a 64-digit hexadecimal key.",
        tr: "8–63 karakterlik Wi-Fi parolası veya 64 haneli onaltılık anahtar kullanın."
    },
    VncPasswordTooLongError {
        en: "The VNC password may contain at most 8 characters.",
        tr: "VNC parolası en fazla 8 karakter içerebilir."
    },
    EmptyPasswordError {
        en: "The account password cannot be empty.",
        tr: "Hesap parolası boş olamaz."
    },
    PasswordGenerationError {
        en: "The password could not be protected on this system. Restart the application and try again.",
        tr: "Parola bu sistemde güvenli biçimde işlenemedi. Uygulamayı yeniden başlatıp tekrar deneyin."
    },
    UserAccountConfigured { en: "• User account configured", tr: "• Kullanıcı hesabı yapılandırıldı" },
    WifiConfigured { en: "• Wi-Fi configured", tr: "• Wi-Fi yapılandırıldı" },
    HostnameConfigured { en: "• Hostname configured", tr: "• Makine adı yapılandırıldı" },
    KeymapConfigured { en: "• Keymap configured", tr: "• Klavye düzeni yapılandırıldı" },
    TimezoneConfigured { en: "• Timezone configured", tr: "• Saat dilimi yapılandırıldı" },
    SshKeyConfigured { en: "• SSH key configured", tr: "• SSH anahtarı yapılandırıldı" },
    UsbDhcpEnabled { en: "• USB DHCP enabled", tr: "• USB DHCP etkinleştirildi" },

    // ---- Notifications and generic failures -------------------------------------------
    FlashCancelledNotification { en: "Flashing cancelled by user", tr: "Yazma kullanıcı tarafından iptal edildi" },
    DownloadCancelledNotification { en: "Download cancelled by user", tr: "İndirme kullanıcı tarafından iptal edildi" },
    FlashFailedNotification { en: "Flashing failed", tr: "Yazma başarısız" },
    DownloadFailedNotification { en: "Download failed", tr: "İndirme başarısız" },
    FlashFinishedNotification { en: "Flashing finished successfully", tr: "Yazma başarıyla tamamlandı" },
    DownloadFinishedNotification { en: "Download finished successfully", tr: "İndirme başarıyla tamamlandı" },
    GenericFlashFailedBody {
        en: "The operation could not be completed. Check that the card is still connected and writable, then try again. Technical details are available in Logs.",
        tr: "İşlem tamamlanamadı. Kartın hâlâ bağlı ve yazılabilir olduğunu denetleyip yeniden deneyin. Teknik ayrıntılar Günlükler bölümündedir."
    },

    // ---- Application info ---------------------------------------------------------------
    CacheDirectory { en: "Cache Directory", tr: "Önbellek dizini" },
    LogFile { en: "Log File", tr: "Günlük dosyası" },
}

/// Messages that carry values.
///
/// Each is a function rather than a format template so the argument order can differ per
/// language without the call site knowing, and so every one can be snapshot-tested.
pub mod fmt {
    use super::Lang;

    /// The confirmation shown before a write destroys whatever is on the destination.
    ///
    /// `destination` is the human-readable name of the disk, e.g. `"SDXC Card (31.9 GB)"`.
    pub fn erase_confirm_body(lang: Lang, destination: &str) -> String {
        match lang {
            Lang::En => format!(
                "Everything on {destination} will be erased and cannot be recovered. Make sure this is the right card."
            ),
            Lang::Tr => format!(
                "{destination} üzerindeki her şey silinecek ve geri getirilemeyecek. Doğru kartı seçtiğinizden emin olun."
            ),
        }
    }

    /// Shown when the destination is smaller than the image needs.
    pub fn destination_too_small_body(lang: Lang, needed: &str, available: &str) -> String {
        match lang {
            Lang::En => format!(
                "This image needs {needed} but the card holds only {available}. Use a larger card."
            ),
            Lang::Tr => format!(
                "Bu imaj {needed} alan gerektiriyor; kartta yalnızca {available} var. Daha büyük bir kart kullanın."
            ),
        }
    }

    /// Shown on the app-info screen and in the window title.
    pub fn version_line(lang: Lang, app_name: &str, version: &str) -> String {
        match lang {
            Lang::En => format!("{app_name} v{version}"),
            Lang::Tr => format!("{app_name} s{version}"),
        }
    }

    /// Shown when a newer release is available.
    pub fn update_available(lang: Lang, version: &str) -> String {
        match lang {
            Lang::En => format!("A new version of the application is available: {version}"),
            Lang::Tr => format!("Uygulamanın yeni bir sürümü mevcut: {version}"),
        }
    }

    /// Shown while the saved catalog is in use, with how old it is.
    pub fn catalog_age(lang: Lang, age: &str) -> String {
        match lang {
            Lang::En => format!("Saved image list, {age} old"),
            Lang::Tr => format!("Kayıtlı imaj listesi, {age} önce alınmış"),
        }
    }

    /// The one-line summary of where a failed write stopped.
    ///
    /// `stage` is already localised — pass `lang.text(Msg::FlashingImage)` and friends.
    pub fn failed_during(lang: Lang, stage: &str) -> String {
        match lang {
            Lang::En => format!("Stopped during: {stage}"),
            Lang::Tr => format!("Şu adımda durdu: {stage}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_has_both_languages() {
        // The macro guarantees a Turkish arm exists; this guards against one that was "filled in"
        // by pasting the English string, which the compiler cannot catch.
        //
        // The exceptions are strings that are genuinely identical in both languages: acronyms and
        // proper nouns. Listing them here means adding a new one is a deliberate act.
        const IDENTICAL_BY_DESIGN: &[Msg] = &[Msg::Ssid];

        for &msg in Msg::ALL {
            let en = Lang::En.text(msg);
            let tr = Lang::Tr.text(msg);

            assert!(!en.is_empty(), "{msg:?} has an empty English string");
            assert!(!tr.is_empty(), "{msg:?} has an empty Turkish string");

            if !IDENTICAL_BY_DESIGN.contains(&msg) {
                assert_ne!(
                    en, tr,
                    "{msg:?} was not translated — Turkish equals English"
                );
            }
        }
    }

    #[test]
    fn language_codes_round_trip() {
        for lang in Lang::ALL {
            assert_eq!(Lang::from_code(lang.code()), Some(lang));
        }
    }

    #[test]
    fn language_picker_uses_native_names() {
        assert_eq!(Lang::En.native_name(), "English");
        assert_eq!(Lang::Tr.native_name(), "Türkçe");
        assert_eq!(Lang::Tr.to_string(), "Türkçe");
    }

    #[test]
    fn locale_tags_resolve_to_their_base_language() {
        assert_eq!(Lang::from_code("tr_TR"), Some(Lang::Tr));
        assert_eq!(Lang::from_code("tr-TR.UTF-8"), Some(Lang::Tr));
        assert_eq!(Lang::from_code("en_GB"), Some(Lang::En));
        assert_eq!(Lang::from_code("EN"), Some(Lang::En));
    }

    #[test]
    fn unsupported_locales_are_reported_rather_than_defaulted() {
        // `from_code` must not quietly answer "English" — the caller decides what an unknown
        // locale means, and today that decision is "use the default and log it".
        assert_eq!(Lang::from_code("de_DE"), None);
        assert_eq!(Lang::from_code(""), None);
        assert_eq!(Lang::from_code("nonsense"), None);
    }

    /// Snapshot of every interpolated message in both languages (`instruction.md` §11.2).
    ///
    /// Spelled out rather than generated, so a wording change has to be made deliberately in the
    /// test as well as the catalog, and a swapped argument in one language shows up as a diff.
    #[test]
    fn interpolated_messages_snapshot() {
        assert_eq!(
            fmt::erase_confirm_body(Lang::En, "SDXC Card (31.9 GB)"),
            "Everything on SDXC Card (31.9 GB) will be erased and cannot be recovered. Make sure this is the right card."
        );
        assert_eq!(
            fmt::erase_confirm_body(Lang::Tr, "SDXC Kart (31,9 GB)"),
            "SDXC Kart (31,9 GB) üzerindeki her şey silinecek ve geri getirilemeyecek. Doğru kartı seçtiğinizden emin olun."
        );

        assert_eq!(
            fmt::destination_too_small_body(Lang::En, "8.0 GB", "4.0 GB"),
            "This image needs 8.0 GB but the card holds only 4.0 GB. Use a larger card."
        );
        assert_eq!(
            fmt::destination_too_small_body(Lang::Tr, "8,0 GB", "4,0 GB"),
            "Bu imaj 8,0 GB alan gerektiriyor; kartta yalnızca 4,0 GB var. Daha büyük bir kart kullanın."
        );

        assert_eq!(
            fmt::version_line(Lang::En, "T3 Gemstone Imager", "1.0.13"),
            "T3 Gemstone Imager v1.0.13"
        );
        assert_eq!(
            fmt::version_line(Lang::Tr, "T3 Gemstone Imager", "1.0.13"),
            "T3 Gemstone Imager s1.0.13"
        );

        assert_eq!(
            fmt::update_available(Lang::En, "1.1.0"),
            "A new version of the application is available: 1.1.0"
        );
        assert_eq!(
            fmt::update_available(Lang::Tr, "1.1.0"),
            "Uygulamanın yeni bir sürümü mevcut: 1.1.0"
        );

        assert_eq!(
            fmt::catalog_age(Lang::En, "2 days"),
            "Saved image list, 2 days old"
        );
        assert_eq!(
            fmt::catalog_age(Lang::Tr, "2 gün"),
            "Kayıtlı imaj listesi, 2 gün önce alınmış"
        );

        assert_eq!(
            fmt::failed_during(Lang::En, Lang::En.text(Msg::FlashingImage)),
            "Stopped during: Flashing Image ..."
        );
        assert_eq!(
            fmt::failed_during(Lang::Tr, Lang::Tr.text(Msg::FlashingImage)),
            "Şu adımda durdu: İmaj yazılıyor ..."
        );
    }

    /// Every value handed to an interpolated message must survive into the output.
    ///
    /// A translation that drops its `{}` still compiles and still reads plausibly; this catches
    /// the case where the Turkish sentence was rewritten and the value fell out of it.
    #[test]
    fn interpolated_messages_never_drop_their_values() {
        for lang in Lang::ALL {
            assert!(fmt::erase_confirm_body(lang, "SENTINEL").contains("SENTINEL"));
            assert!(fmt::update_available(lang, "SENTINEL").contains("SENTINEL"));
            assert!(fmt::catalog_age(lang, "SENTINEL").contains("SENTINEL"));
            assert!(fmt::failed_during(lang, "SENTINEL").contains("SENTINEL"));

            let too_small = fmt::destination_too_small_body(lang, "NEEDED", "AVAILABLE");
            assert!(too_small.contains("NEEDED"), "{lang:?}: needed was dropped");
            assert!(
                too_small.contains("AVAILABLE"),
                "{lang:?}: available was dropped"
            );

            let version = fmt::version_line(lang, "APPNAME", "VERSION");
            assert!(version.contains("APPNAME"));
            assert!(version.contains("VERSION"));
        }
    }

    /// The destructive-action confirmation must name the destination in both languages.
    ///
    /// This is the one message where a generic "are you sure?" is a real hazard: it is the last
    /// thing between the user and an erased disk, so it has to say *which* disk.
    #[test]
    fn erase_confirmation_names_the_destination_in_both_languages() {
        for lang in Lang::ALL {
            let body = fmt::erase_confirm_body(lang, "Kingston DataTraveler");
            assert!(
                body.contains("Kingston DataTraveler"),
                "{lang:?}: the confirmation does not say which disk is about to be erased"
            );
        }
    }
}
