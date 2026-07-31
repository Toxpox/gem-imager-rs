# Faz 4 — Güvenilir SD yazma ve read-back raporu

`instruction.md` §9 gereği. Tarih: 2026-08-01. Host: Windows 11, Git Bash + PowerShell.
Başlangıç: `a9fa8dc` (Faz 3 sonu) · Bitiş: `5292474` · Dal: `t3-gemstone-imager`

---

## 0. Faz 3 doğrulaması

| Kriter | Durum | Kanıt |
|---|---|---|
| Extract boyut + sha256 sert kapı | ✅ | `bb-flasher/src/img/verify.rs`, faz başında yeşil |
| Downloader transport politikası | ✅ | `bb-downloader/src/policy.rs` |
| Last-known-good katalog cache'i | ✅ | `bb-config/src/t3/store.rs` |
| Faz 3 test tabanı | ✅ | `cargo test --all-features --workspace` faz başında geçiyordu |

Faz 3'ün devrettiği açık maddelerden hiçbiri bu fazın kapsamında değildi; hiçbiri kapanmadı.
Faz 4 katalog, DB ve downloader yüzeylerine dokunmuyor.

---

## 1. §9 — SD boru hattı

`instruction.md` §9'daki boru hattı artık kodda birebir bu sırada:

```
doğrulanmış archive → streaming XZ decode → extract hash/boyut (Faz 3)
→ write_all + bayt sayımı + akış hash'i          bb-flasher-sd/src/flashing/mod.rs::writer_task
→ commit (flush + OS/device sync)   ← sert hata  helpers::Commit
→ yazılan raw bölgeyi geri oku ve hash'le        flashing::verify_written
→ karşılaştır                       ← sert hata
→ config.ini yaz (customization)
→ commit                            ← sert hata
→ eject (best-effort)
```

Faz 4 öncesi **üç yol sessizce "başarılı" dönüyordu**:

| # | Sessiz başarısızlık | Şimdi |
|---|---|---|
| 1 | İmaj akışı bildirilen boyuttan önce biterse yazılan bayt sayılmıyordu | `Error::ShortWrite` |
| 2 | `let _ = sd.into_inner().eject()` — sync hatası yutuluyordu | `Error::SyncFailed` |
| 3 | Cihaz kabul ettiği baytları saklamazsa hiçbir kapı görmüyordu | `Error::ReadBackMismatch` |

3 numara kritik: arşiv ve extract hash'leri yalnız cihaza *giden* veriyi tanımlar. Bozuk/sahte
kart, kopan kablo ve controller cache'i bu kapıların hepsinden geçer.

### Doğrulama zinciri

Read-back, **yazıcının cihaza verdiği baytların** hash'iyle karşılaştırılıyor. Bu baytların
katalog `extract_sha256`'sına eşitliği Faz 3'ün `ExtractVerifier`'ında decoder çıkışında zaten
zorlanıyor. Zincir böylece kapanıyor:

```
katalog extract_sha256 → decoder (Faz 3) → yazıcı akış hash'i (Faz 4) → cihaz read-back (Faz 4)
```

Yazıcı hash'i, son chunk'a `read_aligned`'ın eklediği 512 hizalama padding'ini de kapsar; yani
`extract_sha256` ile bit-bit aynı değil, **yazılan bölgenin** hash'idir. Read-back tam olarak aynı
bölgeyi okur, dolayısıyla karşılaştırma tutarlıdır.

### `SdCardWrapper` cache'inin emekliye ayrılması

Windows güvenilirliği için ilk 4 KiB blok en sona erteleniyor. `finish()` çağrılmadan yapılan bir
read-back blok 0'ı **kendi yazma tamponuyla** karşılaştırırdı — kendini doğrulardı. `SdCardWrapper`
artık `finished` bayrağı taşıyor: `commit()` önce `finish()`'i çalıştırıp bloğu cihaza yazıyor,
sonra cache emekliye ayrılıyor ve tüm okuma/yazmalar doğrudan `inner`'a gidiyor.

---

## 2. §9.1 — Hedef güvenliği

| §9.1 maddesi | Durum | Nerede |
|---|---|---|
| Sistem diskini reddet | ✅ | `flashing::evaluate_target` → `Error::SystemDisk` |
| Kapasite `extracted_size` için yeterli mi (yazmadan önce) | ✅ | `flash_internal` → `Error::InsufficientCapacity` |
| Kullanıcıya model, kapasite, stabil kimlik göster | ✅ (mevcut) | `Device { name, path, size }`, GUI destination ekranı |
| Removable bilgisine tek başına güvenme | ✅ | `Device.is_system` eklendi; `is_removable` tek ölçüt değil |
| Çıkarılan hedef → short write / flush hatası başarıya dönüşmesin | ✅ | `ShortWrite`, `SyncFailed`, read-back `UnexpectedEof` |
| Mount edilen kritik partition'ları kontrollü unmount et | ❌ | **Uygulanmadı** — §5.1 |

`is_removable` tek başına yetmiyor: USB'ye takılı sistem diskleri ve dahili kart okuyucular en az
bir platformda removable raporluyor. Karar `bb_drivelist`'in kendi `is_system` tespitine bağlandı
ve `bb_flasher_sd::Device`'a taşındı.

Hedef kararı saf fonksiyona (`evaluate_target(Option<&Device>)`) ayrıldı; böylece donanımsız test
edilebiliyor. `guard_target` yalnız enumerate edip bu fonksiyonu çağırıyor.

---

## 3. §9.2 — Doğrulama politikası

Tam raw read-back **varsayılan ve tek mod**. Hızlı mod eklenmedi — §9.2 hızlı modun varsayılan
olamayacağını söylüyor, bu yüzden hiç eklenmemesi tercih edildi. "Tam doğrulandı" metni yalnız
gerçekten geri okunan bölge için kullanılıyor (`instruction.md` §1/9. madde).

### Ayrık aşama ilerlemesi

`bb_flasher_sd::Status` eklendi:

```rust
pub enum Status { Preparing, Writing(f32), Verifying(f32), Customizing }
```

`DownloadFlashingStatus::Verifying` → `Verifying(f32)` oldu. Gerekçe: tam read-back, doğruladığı
yazma kadar sürüyor. Önceki kod GUI'de `Verifying` için sabit `0.99` gösteriyordu — %100'e varıp
dakikalarca kıpırdamayan çubuk, kullanıcının kartı erken çekmesinin başlıca sebebi.

İptal artık dört aşamanın hepsinde çalışıyor: download (mevcut), write, verify (yeni),
customize (yeni — önceden `c.customize(&mut sd, None)` ile token hiç geçmiyordu).

---

## 4. §9.3 test matrisi

| §9.3 maddesi | Test | Durum |
|---|---|---|
| `mock_sd` tam yazma ve read-back | `mock_card::a_full_flash_writes_verifies_and_customizes` | ✅ |
| Short write enjeksiyonu | `a_stream_shorter_than_the_declared_size_is_a_short_write`, `a_truncated_image_fails_instead_of_succeeding` | ✅ |
| Flush/sync hata enjeksiyonu | `mock_card::a_sync_failure_fails_the_flash` | ✅ |
| Yazma sonrası tek byte flip → hash mismatch | `read_back_rejects_a_single_flipped_byte` | ✅ |
| Hedef çıkarma; yanlış cihaza devam edilmemesi | `read_back_rejects_a_device_that_returns_too_little` | ⚠ kısmi — kısa okuma kapsanıyor, fiziksel yeniden takma değil |
| Yetersiz kapasite | `mock_card::an_image_larger_than_the_card_is_rejected_before_writing` + `..._exactly_fills_the_card_is_accepted` | ✅ |
| MBR + birinci FAT partition tespiti | `mock_card::a_full_flash_...` (gerçek 128 MiB MBR+FAT32 imajı), `tests/customize.rs` | ✅ |
| İptal: download / write / verify / customize | `flash_aborts_with_cancelled_token`, `read_back_stops_when_cancelled`, `test_cancellation_token` | ⚠ kısmi — write ve verify kapsanıyor, customize aşaması için ayrı test yok |

Ek testler (§9.3'te yok, regresyon için eklendi):

- `progress_never_exceeds_one_when_the_last_chunk_is_padded` — 512 padding'i progress'i 1.0'ın
  üstüne çıkarmıyor; GUI'nin ETA ekstrapolasyonu (`state.rs::time_remaining_from`) negatif süre
  üretmesin diye.
- `read_back_reports_its_own_progress_stage` — verify kendi aşamasını yayınlıyor, write yaymıyor.
- `stages_are_reported_in_pipeline_order` — aşamalar geri gitmiyor, verify write'la iç içe geçmiyor.
- `test_public_flash_with_temp_file` artık write'ın %100'e ulaşmasını **ve** bir `Verifying`
  aşamasının raporlanmasını şart koşuyor — doğrulanmamış yazmanın testten geçmesi imkânsız.
- `bb-flasher/src/flasher/sd/mod.rs::status_tests` — `Status` → `DownloadFlashingStatus` eşlemesi;
  verify'ın write çubuğuna geri katlanmadığını doğruluyor.
- `target_guard::{a_system_disk_is_refused, a_normal_card_reports_its_capacity,
  an_unknown_capacity_does_not_block_the_flash}`.

### Ölçülen sonuçlar

```
cargo fmt --all -- --check                                  → temiz
cargo check --all-targets --all-features --workspace        → 0 hata
cargo check --all-targets -p bb-imager-cli --features dfu   → 0 hata
cargo check --all-targets -p bb-imager-gui --features sd -F updater,pre-release
                                                            → 0 hata, 1 uyarı
                                                              (mevcut: helpers.rs:601 unused `body`)
cargo test -p bb-flasher-sd --features mock_sd              → 29 geçti / 4 suite
cargo test --all-features --workspace                       → 216 geçti / 26 suite
cargo clippy --all-targets --all-features --workspace       → 0 hata, 9 uyarı
                                                              (hepsi mevcut: bb-drivelist ve
                                                              dokunulmayan pal/windows.rs:132)
```

**Çalıştırılamayanlar:**

- `make check` / `make test` — `make` bu host'un PATH'inde yok. Makefile'ın feature setleri
  (`_RUST_ARGS_CLI = --features dfu`, `_RUST_ARGS_GUI = --features sd`, satır 157-165) elle
  yukarıdaki `cargo` komutlarına çevrildi.
- `cargo deny check` — `cargo-deny` kurulu değil. Bu faz iki bağımlılık ekliyor (`sha2 = "0.10"`,
  `const-hex = "1.19"`); **ikisi de workspace'te zaten kullanılıyor** (`bb-flasher`, `bb-config`),
  yeni lisans/advisory yüzeyi beklenmiyor ama doğrulanmadı.

### Platform durumu

- **Windows:** derlendi, 216 test geçti. `Commit for WinDrive` aktif yolda.
- **Linux:** **derlenmedi.** `rustup target list --installed` yalnız `x86_64-pc-windows-msvc`
  döndürüyor. `pal/linux.rs`'e eklenen `Commit for LinuxDrive` hiçbir derleyiciden geçmedi.
- **macOS:** **derlenmedi.** `Commit for MacOSFile` aynı durumda.

Her iki impl de `windows.rs`'tekiyle aynı üç satırlık kalıp, ama derlenmiş sayılmamalı.

### Gerçek kart

**Yapılmadı.** Hiçbir fiziksel T3-GEM-O1 veya SD kart testi çalıştırılmadı. Tüm doğrulama `MockSd`
(tempfile tabanlı 128 MiB MBR+FAT32 imajı) ve `Cursor` üzerinden yapıldı. Read-back'in O_DIRECT
altında gerçek blok cihazlardaki davranışı — özellikle hizalama gereksinimleri ve son chunk'ın
`next_multiple_of(4096)` ile yuvarlanması — donanımda kanıtlanmadı.

---

## 5. Açık kalanlar

### 5.1 Kontrollü unmount yok — Faz 4'ün kapatmadığı §9.1 maddesi

Mount edilmiş kritik partition'lar yazmadan önce unmount edilmiyor; "başarısız unmount sonrası
yazma yapma" kuralı uygulanmıyor. Gereken: Linux'ta udisks2, Windows'ta volume lock
(`FSCTL_LOCK_VOLUME`), macOS'ta `diskutil unmountDisk` — `pal` katmanına ayrı bir PR.
Şu an tek savunma katmanı `is_system`.

### 5.2 `config.ini` FAT read-back'i

§9 boru hattındaki "FAT'tan config.ini geri oku" adımı **Faz 5 kapsamında** (serializer oradan
geliyor). Şu an customization sonrası yalnız `commit()` yapılıyor, içerik geri okunmuyor.

### 5.3 Linux/macOS derlemesi

Üç platform derlemesi CI kapısı olarak çalıştırılmalı (§4 "Platform durumu").

### 5.4 `guard_target` cihazı bulamazsa geçiyor

Drive list enumeration izin veya egzotik transport nedeniyle cihazı kaçırabilir. Bulamama "güvenli"
kanıtı değil, ama reddetmek gerçek donanımı bloke ederdi. Şu an `tracing::warn!` ile loglanıyor ve
kapasite + sistem-disk kontrolleri atlanıyor. Kararın gözden geçirilmesi gerekebilir.

### 5.5 Customize aşaması iptali için ayrı test yok

Token artık `c.customize()`'a geçiyor ve `check_cancel` döngüde çağrılıyor, ama bu yolu izole eden
bir test yazılmadı.

---

## 6. Commit serisi

```
5292474  feat(sd): add byte-count and read-back verification
```

Faz 0-3'ün aksine bu faz **tek koda ait commit** içeriyor. Bölmek denendi ve bırakıldı: `lib.rs`
beş `Error` varyantının hepsini, `flashing/mod.rs` ise bayt sayımı + read-back + hedef kapısını
birlikte taşıyor. Dosya seviyesinde bölünce ne mantıksal ayrım çıkıyor ne de ara commit'ler derleniyor
— `bb_flasher_sd::Status` ve `DownloadFlashingStatus::Verifying(f32)` değişiklikleri tüketicileriyle
aynı commit'te olmak zorunda. "Her commit derlenir" kuralı, "her commit tek amaç taşır" kuralına
tercih edildi.

Kırılan iki public API ve tüketicileri (aynı commit içinde):

1. `bb_flasher_sd::flash()` progress kanalı `SyncSender<f32>` → `SyncSender<Status>`
   (tüketici: `bb-flasher/src/flasher/sd/mod.rs`).
2. `DownloadFlashingStatus::Verifying` → `Verifying(f32)`
   (tüketiciler: `bb-imager-cli/src/lib.rs`, `bb-imager-gui/src/ui/flash.rs`,
   `bb-imager-gui/src/state.rs`).

`Cargo.lock` `sha2`/`const-hex` nedeniyle güncellendi; ilk commit'e girmeli.

### Değişen dosyalar

```
bb-flasher-sd/Cargo.toml                    sha2 + const-hex
bb-flasher-sd/src/lib.rs                    Error varyantları, Device.is_system, Status export
bb-flasher-sd/src/helpers.rs                Commit trait, SdCardWrapper.finished, read_at_least,
                                            progress clamp, chan_send genelleştirme
bb-flasher-sd/src/flashing/mod.rs           Status, WriteOutcome, verify_written, guard_target,
                                            evaluate_target, flash_internal boru hattı
bb-flasher-sd/src/flashing/tests.rs         yeni testler
bb-flasher-sd/src/mock_sd.rs                sync_fail_token, Commit
bb-flasher-sd/src/pal/linux.rs              Commit for LinuxDrive      (derlenmedi)
bb-flasher-sd/src/pal/macos.rs              Commit for MacOSFile       (derlenmedi)
bb-flasher-sd/src/pal/windows.rs            Commit for WinDrive
bb-flasher-sd/tests/flashing.rs             aşama sırası + truncated imaj testleri
bb-flasher/src/common.rs                    Verifying(f32)
bb-flasher/src/flasher/sd/mod.rs            translate_status + status_tests
bb-imager-cli/src/lib.rs                    verify progress çubuğu
bb-imager-gui/src/ui/flash.rs               verify aşaması gerçek kesir
bb-imager-gui/src/state.rs                  test güncellemesi
Cargo.lock                                  sha2, const-hex
```

### Çalışma ağacındaki ilgisiz değişiklikler

`cargo fmt --all` workspace genelinde çalıştığı için Faz 4 ile ilgisiz **4 dosyada** saf
biçimlendirme değişikliği bıraktı (toplam +9/−5, davranış değişikliği yok):

```
bb-helper/src/lib.rs               modül sırası
bb-helper/src/reader_progress.rs   fazladan boş satır
bb-imager-gui/src/constants.rs     satır sarma
bb-imager-gui/src/message.rs       argüman sarma
```

Bunlar Faz 4 commit'lerine **girmemeli**; ayrı bir `style:` commit'i veya `git checkout --` ile
geri alınmalı. (Geri alma bu oturumda denendi, izin katmanı tarafından reddedildi.)
