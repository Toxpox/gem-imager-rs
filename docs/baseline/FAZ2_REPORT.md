# Faz 2 — Kontrollü budama raporu

`instruction.md` §7 gereği. Tarih: 2026-07-31. Host: Windows 11, Git Bash + PowerShell.
Başlangıç: `8c67713` (Faz 1 sonu) · Bitiş: `bf1140b`

---

## 0. Faz 0 ve Faz 1 doğrulaması

Faz 2'ye başlamadan önce §7'nin ön koşulu ("Budamaya yalnız Faz 0 fixture'ları ve
Faz 1 canonical model testleri yeşilken başla") kontrol edildi.

### 0.1 Faz 0 (§5.4 çıkış kapısı)

| Kriter | Durum | Kanıt |
|---|---|---|
| Baseline raporu kayıtlı | ✅ | `docs/baseline/FAZ0_BASELINE_REPORT.md` + 10 ham `.log` |
| T3 SD ve DFU sözleşmesi sabit | ✅ | `bb-config/tests/fixtures/t3/{main_catalog,boot_manifest}.json`, `dfu_contract.md`, `invalid/*.json` |
| Ürün kapsamı ADR'si | ✅ | `docs/adr/0001-product-scope.md` (`T3Only`, güvenli varsayılan) |
| Test durumu yeniden üretilebilir | ✅ | Komutlar §3'te birebir listelenmiş; bu oturumda tekrar çalıştırıldı |

FAZ0'ın açık bıraktığı 4 eksikten **1'i bu fazda kapatıldı**: release binary
boyutu artık hem budama öncesi hem sonrası ölçüldü (§4). Kalan 3'ü hâlâ açık:
`bb-imager-gui` test binary'sinin UAC hatası, host'ta `make`/`cargo-deny`/
`cargo-packager` bulunmaması, fixture'ların canlı servisle çapraz doğrulanmamış
olması.

### 0.2 Faz 1 (§6.5 testleri)

`cargo test -p bb-config` — **63 test, hepsi geçti**. Test adları §6.2'nin
zorunlu invariantlarını birebir karşılıyor; örnekler:

- `an_empty_catalog_is_an_error_not_a_success` — boş sonuç başarı sayılmıyor
- `all_four_integrity_gates_are_stored_separately` — dört bütünlük kapısı ayrı
- `a_non_t3_board_claiming_emmc_does_not_get_a_dfu_profile` — `emmc: true` yalnız T3'te DFU profili üretiyor
- `https_downgrade_is_rejected`
- `every_t3_image_in_the_live_catalog_binds_correctly`
- `unknown_fields_are_ignored_for_forward_compatibility` (pozitif)
- `a_database_from_a_newer_build_is_refused_with_a_diagnostic`
- store/migration: `a_fresh_store_is_migrated_to_the_current_schema_version`,
  `a_corrupt_hash_column_is_reported_instead_of_being_silently_accepted`

`cargo fmt -p bb-config -- --check`: temiz.

### 0.3 ⚠️ Doğrulamada bulunan iki gerçek sorun

1. **`cargo fmt --all -- --check` upstream'de zaten kırık.** 13 dosyada rustfmt
   farkı var (`bb-bmap-parser/tests/*`, `bb-downloader/tests/test.rs`,
   `bb-helper/src/*`, `bb-imager-gui/src/{constants,message}.rs`, …). Hiçbiri
   Faz 1'de yazılan `t3` modülünde değil — **rustfmt sürüm sapmasından gelen
   pre-existing durum**. `instruction.md` §16 bunu her PR kapısı sayıyor;
   CI'ya alınmadan önce ayrı bir `style:` PR'ıyla düzeltilmeli. Bu fazda
   dokunulmadı (§4.2: ilgisiz dosyaları biçimlendirme).

2. **`make test` doctest çalıştırmıyor.** Makefile `test` hedefi
   `cargo test --all-targets` kullanıyor; bu bayrak doctest'leri atlar. Bu
   yüzden `bb-flasher`'ın crate-level doc örneğinin **upstream'de kırık olduğu**
   fark edilmemiş: artık var olmayan bir `BBFlasher` trait'ini import ediyor,
   `into_image_future()` çağırıyor ve `without_bmap`'e 4 argüman veriyordu.
   `ca47dc1` bunu düzeltti; `cargo test --doc -p bb-flasher -F sd` artık geçiyor.
   **CI kapısına ayrıca `cargo test --doc` eklenmeli.**

> **Faz 1'in kapanmamış işi (Faz 2'nin parçası değil):** `bb_config::t3`
> adaptörü ve `t3::store` yazıldı ve testli, ama **GUI'ye bağlanmadı**. GUI hâlâ
> eski `bb_config::config` + `tempfile::NamedTempFile` yolunu kullanıyor. Bu
> bağlantı `instruction.md`'de ayrı numaralı bir faz değil; Faz 3/4 ile birlikte
> yapılması gerekiyor ve **T3 imajlarını arayüzde görmenin ön koşuludur**.

---

## 1. Budama kapsamı ve commit serisi

§7 "Her crate veya bağımlılık grubu ayrı commit olmalı" kuralına göre 7 commit:

| Commit | Kapsam | Δ satır |
|---|---|---:|
| `31df758` | `bb-flasher-pb2-mspm0` + PocketBeagle 2 yolu | −659 |
| `8ad1172` | `bb-flasher-mspm0` + BeagleConnect Zepto yolu | −1.747 |
| `9acc32c` | `bb-flasher-bcf` + BeagleConnect Freedom yolu | −1.657 |
| `ca47dc1` | `bb-bmap-parser` + bmap writer yolu | −1.191 |
| `d1892c8` | `SdCardBootfs` / `UpdateBootFlasher` + `OsArchive` | −865 |
| `e99faf1` | Kapsam dışı kart asset'leri ve remote config'ler | −117 |
| `bf1140b` | Tek varyantlı `Flasher` sonrası ulaşılamaz match kolları | −7 |
| **Toplam** | 88 dosya | **+108 / −6.203** |

Her commit DCO sign-off taşıyor (`git commit -s`).

### 1.1 Silinen crate'ler

| Crate | Gerekçe |
|---|---|
| `bb-flasher-bcf` | BeagleConnect Freedom CC1352P7 + MSP430 — kapsam dışı |
| `bb-flasher-mspm0` | TI MSPM0 BSL (UART/I2C), BeagleConnect Zepto — kapsam dışı |
| `bb-flasher-pb2-mspm0` | PocketBeagle 2 yardımcı işlemcisi — kapsam dışı |
| `bb-bmap-parser` | T3 kataloğu hiç `.bmap` yayınlamıyor (34/34 imajda alan yok) |

**Workspace 14 → 10 crate.** Kalan: `bb-config`, `bb-downloader`, `bb-drivelist`,
`bb-flasher`, `bb-flasher-dfu`, `bb-flasher-sd`, `bb-helper`, `bb-iced-widgets`,
`bb-imager-cli`, `bb-imager-gui`.

### 1.2 Silinen kod yolları

- **`bb-flasher::{bcf, mspm0, pb2}`** façade modülleri
- **bmap yazma yolu**: `writer_task_bmap`, `flash`/`write_sd`/`flash_internal`
  imzalarındaki `Option<B>` parametresi, `Flasher<I, B>`'nin ikinci tip
  parametresi ve `without_bmap` yapıcısı, `Error::InvalidBmap`
- **`SdCardBootfs` / `UpdateBootFlasher`**: `bb_flasher_sd::bootfs_update` ve
  onu besleyen `OsArchive` tar okuyucusu (+ `LocalImage::into_archive_fn`,
  GUI'nin iki `into_archive_fn` sarmalayıcısı)
- **CLI alt komutları**: `bcf`, `msp430`, `zepto`, `sd-boot-update`,
  `--bmap` bayrağı ve onun için var olan `LocalStringFile` yardımcısı;
  `DestinationsTarget`'ın `Bcf`/`Msp430`/`Zepto` varyantları
- **GUI**: `Destination::{BeagleConnectFreedom, Msp430, Mspm0}`,
  `FlashingCustomization::{Bcf, Msp430, Zepto}`, "Skip Verification" paneli,
  kalıcı `bcf_customization`/`zepto_customization` ayarları, `Bmap` struct'ı
- **`bb-flasher::common`**: yalnız BCF/MSPM0'ın kullandığı `FlasherError` ve
  `resolve_img`
- **Feature'lar**: `bcf`, `bcf_msp430`, `pb2_mspm0`, `mspm0_uart`, `mspm0_i2c`,
  `static_hidraw`, `shared_hidraw` (`bb-flasher`); `bcf_cc1352p7`, `bcf_msp430`,
  `zepto_uart`, `zepto_i2c`, `static-hidraw`, `shared-hidraw` (GUI + CLI).
  **Kalan matris: `sd`, `dfu`, `static`/`system-deps`, `updater`,
  `notify-rust`, `pre-release`, `debug`.**
- **Makefile**: `PB2_MSPM0`, `ZEPTO_I2C`, `BCF_MSP430`, `SHARED_HIDRAW`
  değişkenleri ve `_check_*` reçetelerindeki ilgili feature kombinasyonları
- **CI/paketleme**: `.github/workflows/release.yml` CLI feature listesi,
  `snapcraft.gui.yaml` (`serial-port` plug + değişken geçişleri),
  `snapcraft.cli.yaml` açıklaması

### 1.3 `Flasher` enum'u

`SdCard`, `SdCardBootfs`, `BeagleConnectFreedom`, `Msp430Usb`, `Pb2Mspm0`,
`Mspm0` → **yalnız `SdCard`**.

SQLite discriminant'ları **yeniden atanmadı**: 2–6 boş bırakıldı, böylece eski
bir build'in yazdığı satır sessizce başka bir flasher'a çözülmek yerine
`Invalid Flasher discriminant` hatası verir.

GUI ayar dosyasında (`<config_local_dir>/config.json`) kalan
`bcf_customization`/`zepto_customization` anahtarları geri uyumluluğu kırmaz:
serde bilinmeyen alanları yok sayar, sonraki kaydetmede düşerler.

### 1.4 `config.json` (gömülü varsayılan katalog)

- Cihazlar 7 → 1: yalnız **BeagleY-AI**.
- `remote_configs` 7 → 1: yalnız `beagleboard/distros` (`beagle-am67`
  imajlarını gerçekten sunan tek liste).
- Asset'ler: `assets/boards/{ai64,bbb,bcf,beagleplay,beaglevfire,pocket2}.png`
  silindi; `beagley.png` ve `Template.psd` korundu.

> **ADR 0001 uyumu:** ADR `T3Only` diyor ama §7'nin son maddesi ve ADR'nin
> "Sonuç" bölümü BeagleY-AI'yi **kaynak seviyesinde** korumayı zorunlu kılıyor.
> Bu commit yalnız neyin *paketlendiğini* daraltır, neyin *var olduğunu* değil.
>
> **T3-GEM-O1 bu dosyada bilinçli olarak yok** — T3 kartı ve imajları Faz 1'de
> eklenen katı `bb_config::t3` adaptöründen gelir, bu eski BeagleBoard-şemalı
> config'ten değil.

---

## 2. Düşen bağımlılıklar

`cargo tree --workspace -e normal` ile doğrulandı — hepsi **0 eşleşme**:

| Bağımlılık | Neden düştü |
|---|---|
| `hidapi` / hidraw | `bb-flasher-bcf` ile birlikte |
| `serialport` | `bb-flasher-mspm0` ile birlikte |
| `nix` (I2C, Linux) | `mspm0_i2c` ile birlikte |
| `bin_file` | `pb2_mspm0` ile birlikte |
| `tar` | `OsArchive` ile birlikte (`bb-flasher` + `bb-imager-cli`) |

`rc-zip-sync` **kaldı** — `OsImage` zip'li imajlar için hâlâ kullanıyor.

En önemli sonucu: **static-vs-shared HID linkleme problemi tamamen ortadan
kalktı.** Windows/macOS build matrisini şekillendiren `static-hidraw` /
`shared-hidraw` ikilemi ve "distro hidraw çok eski" istisnası artık yok.

---

## 3. Doğrulama

`make` bu host'ta kurulu değil (FAZ0 §2.1). Makefile'daki `_check_common`,
`_check_cli`, `_check_gui` ve `test` hedefleri satır satır okunup **eşdeğer
cargo komutları** olarak çalıştırıldı. Her commit'ten sonra tekrarlandı.

### 3.1 Son durum (`bf1140b`)

| Hedef | clippy | test |
|---|---|---|
| workspace (bcf/flasher/gui/cli hariç) | 0 hata, 0 uyarı | ✅ |
| `bb-flasher -F dfu,static,piped_image,sd` | 0 hata, 0 uyarı | ✅ |
| `bb-imager-cli --features dfu` | 0 hata, 0 uyarı | ✅ |
| `bb-imager-gui --features sd -F updater,pre-release` | 0 hata, **1 uyarı** | (host'ta UAC) |
| `cargo test --doc -p bb-flasher -F sd` | — | ✅ 1 geçti |

Kalan tek uyarı `bb-imager-gui/src/helpers.rs:562` — `show_notification(body)`
`notify-rust` kapalıyken kullanılmıyor. **Faz 0 baseline'ında da vardı.**

Faz 0'da kayıtlı `bb-drivelist` (8) ve `bb-flasher-sd/pal/windows.rs` (1)
uyarıları da hâlâ duruyor; bunlar bu fazın ürettiği uyarılar değil.

### 3.2 Test sayıları

Budamayla birlikte ilgili testler de silindiği için sayılar düştü; **hiçbir
test başarısız olmadı, hiçbir test görev dışı diye susturulmadı.**

| Paket | Faz 0 | Faz 2 sonu | Not |
|---|---:|---:|---|
| workspace (ortak) | 129 | ~106 | bmap-parser + mspm0 testleri silindi |
| `bb-flasher` | 21 | 21 | değişmedi |
| `bb-imager-cli` | 35 | 26 | zepto/pb2/bmap/sd-boot-update testleri silindi |

CLI'da silinen `sd-boot-update` testlerinin yerine **regresyon testi** kondu:
`sd_boot_update_is_no_longer_a_subcommand` — eski çağrının artık parse
edilmediğini doğruluyor, yani kaldırma sessiz değil görünür.

---

## 4. Binary boyutu (§7 son maddesi)

Faz 0'ın açık bıraktığı ölçüm bu fazda alındı. Windows x86_64, `--release`
(LTO + `codegen-units=1`).

| Binary | Budama öncesi | Budama sonrası | Δ |
|---|---:|---:|---:|
| `bb-imager-cli.exe` | 5.455.360 B (5,20 MiB) | 2.953.728 B (2,82 MiB) | **−45,9 %** |
| `bb-imager-gui.exe` | 26.803.200 B (25,56 MiB) | 24.270.848 B (23,15 MiB) | **−9,4 %** |

> **Dürüst okuma:** bu, aynı feature setinin küçülmesi değil, **release
> reçetesinin ürettiği binary'nin** küçülmesidir. Budama öncesi CLI
> `bcf_cc1352p7,zepto_uart,bcf_msp430,static-hidraw,dfu` ile, sonrası yalnız
> `dfu` ile derlendi — feature matrisinin daralması budamanın amacının
> kendisiydi. GUI'deki kazanç daha küçük çünkü boyutuna iced/wgpu/rusqlite
> hâkim; MCU flasher'ları zaten ufak bir paydı.

---

## 5. §7 kurallarına uyum

| Kural | Durum |
|---|---|
| Her crate/bağımlılık grubu ayrı commit | ✅ 7 commit |
| Silmeden önce `rg` ile tüm referansları bul | ✅ her commit öncesi tam ağaç taraması |
| Workspace üyeleri, `Cargo.lock`, Makefile, CI, paket manifestleri birlikte | ✅ |
| `cargo tree` ile transitive bağımlılıkların düştüğünü doğrula | ✅ §2 |
| Her committe format, check, test | ✅ §3 |
| Binary boyut değişimini raporla | ✅ §4 |
| BeagleY-AI'yi geri döndürülemez silme | ✅ kaynak ağacında korundu |

---

## 6. Bu fazda yapılan iki hata ve düzeltmeleri

Şeffaflık için kaydediliyor:

1. **`cargo fmt --all`** yanlışlıkla budamayla ilgisiz 9 dosyayı da biçimlendirdi
   ve `8ad1172`'ye karıştı (§4.2 ihlali). Dosyalar geri alınıp commit amend
   edildi; sonraki commitlerde yalnız dokunulan dosyalara `rustfmt` uygulandı.
2. **`bb-flasher-sd/src/pal/mod.rs`**'de 6 satırlık bir biçimlendirme farkı
   (bir PostToolUse hook'unun `cargo fmt`'inden) `ca47dc1`'de kaldı. Aynı
   crate'in içi ve davranışsal etkisi yok; ortam `--amend`'i engellediği için
   ayıklanmadı.

---

## 7. Faz 3'e devretmeden önce açık kalanlar

**Bu fazın kapsamında değil, ama izlenmeli:**

1. **`bb_config::t3` GUI'ye bağlı değil** (§0.2 notu). Faz 3/4'ün ilk işi
   olmalı; aksi hâlde uygulama T3 imajlarını hiç göstermez.
2. **`cargo fmt --all -- --check` kırık** (13 dosya, pre-existing). Ayrı
   `style:` PR'ı gerekiyor.
3. **CI'ya `cargo test --doc` eklenmeli** — `make test` doctest atlıyor.
4. **`make`, `cargo-deny`, `cargo-packager` bu host'ta yok.** §16 CI kapıları
   ve Faz 9 paketleme başka bir ortamda doğrulanmalı.
5. **`bb-imager-gui` test binary'si bu Windows host'ta UAC hatası veriyor**
   (Faz 0 baseline failure, hâlâ geçerli).
6. **ADR 0001 `T3Only` diyor**, ancak BeagleY-AI'nin ürün kapsamında olup
   olmadığı teyit edilmedi. Faz 2 her iki okuma altında da aynı sonucu verdiği
   için bu karar Faz 2'yi bloke etmedi; ama **Faz 8 (GUI destination) öncesi
   netleşmeli.**
