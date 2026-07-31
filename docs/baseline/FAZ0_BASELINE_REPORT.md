# Faz 0 — Baseline raporu

`instruction.md` §5.1 gereği kaydedilmiştir. Tarih: 2026-07-31. Host: Windows 11, PowerShell/Git Bash (`rtk` proxy üzerinden).

## 1. Repo yapısı ve HEAD SHA

Repo kökü (`imager-fork/`) git deposu değildir; **iki ayrı git deposu** içerir. `instruction.md` §4.1'e bu bulguya göre madde eklendi.

| Depo | Branch | HEAD SHA | Son commit |
|---|---|---|---|
| `bb-imager-rs/` | `main` | `559a77d163e10f431d18e9e15a9ee60731da7698` | Merge PR #703 (gui-optimize), 2026-07-30 |
| `gem-imager/` | `main` | `ab3570059264324ce38783e51021855b3693ddeb` | "Improve bootloader download: cache fallback, detai...", 2026-07-15 |

`git status --short` her iki depoda da temiz (dirty dosya yok).

## 2. Toolchain

- `rustc 1.97.0` (2d8144b78, 2026-07-07), edition 2024 uyumlu.
- `cargo 1.97.0`.
- `rustfmt 1.9.0-stable`.
- `cargo clippy`: mevcut, sorunsuz.
- `git-lfs 3.5.1` (Windows amd64) — kurulu ve çalışıyor.

### 2.1 Eksik araçlar (ortam boşluğu)

- **`make` PATH'te yok** (mingw32-make/gmake de yok). `bb-imager-rs/CLAUDE.md` ve `instruction.md` §16 CI kapılarının merkezinde `make check`/`make test` var; bu host'ta doğrudan çalıştırılamadı. Bu raporda tüm baseline'lar Makefile'daki `_check_common`/`_check_cli`/`_check_gui` hedeflerinin altındaki gerçek `cargo` komutları elle çıkarılıp çalıştırılarak üretildi (bkz. §3).
- `cargo-packager`, `cargo-deny`, `dx` (Dioxus CLI) kurulu değil. Faz 9 paketleme ve `deny.toml` kontrolü bu host'ta şu an çalıştırılamaz.
- Bu boşluklar Faz 0 çıkışını engellemiyor (check/test cargo ile doğrudan çalıştırılabildi) ama Faz 9 ve CI paritesi için host'a `make`, `cargo-deny`, `cargo-packager` kurulumu önerilir.

## 3. `make check` / `make test` eşdeğeri sonuçlar

`make` yokluğunda Makefile'daki hedefler (`_check_common`, `_check_cli`, `_check_gui`, `test`) satır satır okunup eşdeğer `cargo clippy`/`cargo test` komutları elle çalıştırıldı. Ham loglar `docs/baseline/baseline_*.log` içinde saklanır.

### 3.1 Check (clippy) — tamamı EXIT=0

| Hedef | Sonuç | Pre-existing uyarı |
|---|---|---|
| `_check_common` (workspace − bcf/flasher/gui/cli) | ok | `bb-flasher-sd/src/pal/windows.rs:132` — `needless_borrows_for_generic_args` |
| `_check_common` → `bb-flasher-bcf -F msp430,static` | ok | yok |
| `_check_common` → `bb-flasher -F bcf,bcf_msp430,pb2_mspm0,dfu,static,mspm0_uart,mspm0_i2c,piped_image,sd` | ok | (yukarıdaki bb-flasher-sd uyarısı tekrar) |
| `_check_cli` (`bb-imager-cli` + `bcf_cc1352p7,zepto_uart,dfu,pb2_mspm0,zepto_i2c`) | ok | (yukarıdaki uyarı tekrar) |
| `_check_gui` (`bb-imager-gui` + `bcf_cc1352p7,zepto_uart,sd,updater,zepto_i2c,pre-release`) | ok | `bb-drivelist` lib: 8 uyarı (gereksiz `&mut`, `to_string`/`Display`, collapsible `if`); `bb-imager-gui/src/helpers.rs:765` — `unused_variable: body` |

Hiçbir clippy uyarısı bu oturumda yapılan bir değişiklikten kaynaklanmıyor; workspace HEAD'i hiç dokunulmadan üretildi. Bunlar Faz 0 baseline'ı olarak kaydedildi, gizlenmedi veya "görev dışı" diye silinmedi (§5.1 kuralı).

### 3.2 Test — bir tanesi başarısız (baseline failure, kaydedildi)

| Hedef | Sonuç | Not |
|---|---|---|
| `test` (`_check_common` altında `cargo test`) | ok | Tüm alt-crate testleri geçti (bb-config, bb-downloader, bb-helper, bb-drivelist, bb-flasher-mspm0, bb-flasher-sd, bb-flasher-dfu, bb-bmap-parser dahil) |
| `test-cli` eşdeğeri (bcf, flasher, cli crate'leri) | ok | `bb-flasher-bcf`: 0 test; `bb-flasher` (tüm feature'lar): 13+4+4 test geçti; `bb-imager-cli`: 16+4+15 test geçti |
| `test-gui` eşdeğeri (`bb-imager-gui`) | **BAŞARISIZ — EXIT=101** | Derleme başarılı (1m53s), ancak test binary'si çalıştırılamadı: `Caused by: İstenen işlem için yükseltme gerekiyor. (os error 740)` |

**Baseline failure detayı:** `bb-imager-gui` unit test binary'si bu Windows host'ta yönetici yükseltmesi (UAC elevation) istiyor ve süreç hiç başlatılamıyor (`never executed`). Bu, kod değişikliğinden bağımsız bir host/ortam durumudur — muhtemelen embed edilen Windows executable manifest'i (`embed-resource` crate'i derleme sırasında görüldü) `requireAdministrator` talep ediyor veya bir güvenlik yazılımı test binary'sini engelliyor. Bu Faz 0 sırasında araştırılmadı; ileride GUI testi çalıştırılacak her host için ayrı doğrulanmalı ve gerekirse ya manifest düzeltilmeli ya da test binary'si yükseltilmiş oturumda çalıştırılmalı. Bu proje çalışmasının bir parçası olmadığı için "görev dışı" diye silinmedi; olduğu gibi kaydedildi (ham log: `docs/baseline/baseline_test_gui.log`).

## 4. Workspace crate listesi (`cargo metadata --no-deps`)

| Crate | Sürüm | Feature'lar |
|---|---|---|
| `bb-flasher` | 0.1.0 | bb-flasher-bcf, bb-flasher-dfu, bb-flasher-mspm0, bb-flasher-pb2-mspm0, bb-flasher-sd, bcf, bcf_msp430, default, dfu, mspm0_i2c, mspm0_uart, pb2_mspm0, piped_image, sd, sd_linux_udev, sd_macos_authopen, serde, shared_hidraw, static, static_hidraw |
| `bb-flasher-bcf` | 1.0.2 | cc1352p7, default, msp430, shared_hidraw, static |
| `bb-helper` | 1.0.13 | cancel, file_stream, reader_progress, tokio |
| `bb-flasher-dfu` | 1.0.13 | (yok) |
| `bb-flasher-mspm0` | 1.0.13 | default, i2c, i2cdev, nix, serialport, uart |
| `bb-flasher-pb2-mspm0` | 0.1.0 | (yok) |
| `bb-flasher-sd` | 3.0.0 | macos_authopen, mock_sd, udev |
| `bb-bmap-parser` | 0.1.1 | (yok) |
| `bb-drivelist` | 1.0.0 | (yok) |
| `bb-imager-cli` | 1.0.13 | bcf_cc1352p7, bcf_msp430, default, dfu, pb2_mspm0, shared-hidraw, static-hidraw, zepto_i2c, zepto_uart |
| `bb-imager-gui` | 1.0.13 | bcf_cc1352p7, bcf_msp430, debug, default, notify-rust, pre-release, sd, shared-hidraw, static, static-hidraw, system-deps, updater, zepto_i2c, zepto_uart |
| `bb-config` | 0.1.0 | (yok) |
| `bb-downloader` | 0.2.0 | default, json, native-tls, rustls |
| `bb-iced-widgets` | 1.0.13 | (yok) |

§7 budama listesi (`bb-flasher-bcf`, `bb-flasher-mspm0`, `bb-flasher-pb2-mspm0`, `bb-bmap-parser`) bu listeyle doğrulandı — hepsi gerçekten workspace üyesi.

## 5. git-lfs durumu

`git lfs status`: push/commit/unstaged listesi boş (temiz). `git lfs ls-files`: 19 dosya LFS pointer olarak takip ediliyor (`assets/boards/*.png|.psd`, `assets/os/*.png|.webp`, `assets/screenshots/*.webp`). Hiçbiri "kalan pointer" (yani checkout edilmemiş) durumunda değil.

## 6. Release binary boyutu

**Mevcut değil.** `target/release/` dizini bu checkout'ta boş; yalnız `target/debug/` altında kısmi önceki derleme fingerprint'leri var. Release binary boyutu bu oturumda ölçülmedi — tam release derlemesi (GUI: iced/wgpu/dioxus zinciri, önceki debug check derlemesi ~2 dakika sürdü) bu Faz 0 turunda maliyet/süre nedeniyle çalıştırılmadı. Faz 2 budaması bitiminde "binary boyutundaki değişimi raporla" (§7) gereksinimi için, budama öncesi bir release baseline ölçümü ayrıca alınmalıdır — bu rapor bunu **açık eksiklik** olarak işaretler.

## 7. `bb-config` mevcut şema — Faz 1 için doğrulanmış zemin

`bb-config/src/config.rs` (259 satır) incelendi. Önemli baseline bulguları:

- Mevcut `OsImage` yalnız **tek** hash alanı taşıyor: `image_download_sha256` (arşiv hash'i) + `extract_size` (u64, ayrı hash yok). `instruction.md` §6.1'in istediği `extracted_sha256` alanı bu Rust struct'ında **hiç yok** — bu, `ImageIntegrity` modelinin gerçekten yeni iş olduğunu, mevcut alan yeniden adlandırmasıyla çözülemeyeceğini doğrular.

  > **Faz 1 düzeltmesi (2026-07-31):** Yukarıdaki tespit `bb-config`'in *Rust tipi* için doğrudur, ancak **canlı T3 kataloğu `extract_sha256` alanını yayınlıyor** (34/34 image'da mevcut). Yani dört bütünlük kapısının dördü de sunucu tarafından sağlanıyor; eksik olan yalnız Rust modeliydi. Faz 1'de `bb-config::t3::ImageIntegrity` bu dört alanı ayrı ayrı taşıyacak biçimde eklendi.
- `Config::os_list`, `Imager::devices`, `OsSubList::subitems` üçü de `#[serde_as(as = "VecSkipError<_>")]` kullanıyor — yani şu an geçersiz bir girdi **sessizce atlanıyor**. `instruction.md` §6.2'nin "VecSkipError benzeri sessiz atlama davranışını T3 adaptöründe kullanma" uyarısı gerçek koda dayanıyor; T3 adaptörü bu deseni miras almamalı, ayrı typed diagnostic yolu kurmalı.
- `Flasher` enum'ında `Dfu` varyantı yok (`SdCard`, `SdCardBootfs`, `BeagleConnectFreedom`, `Msp430Usb`, `Pb2Mspm0`, `Mspm0`). Faz 1/6'da eklenecek.

## 8. §2.1 donanım ön koşulu — durum

Bu oturumda T3-GEM-O1 kartı, USB kablosu veya çalışır `gem-imager` kurulumuna erişim **doğrulanmadı** (yazılım-only ortam). §18.2 (resmi ikame) kapısı bu bilgiyle şu an açık kalamaz; §2.1'e göre hedef bilinçli olarak SD-only preview (§18.1) ile sınırlı tutulmalıdır, donanım erişimi netleşene kadar.

## 9. Faz 0 çıkış kapısı değerlendirmesi (§5.4)

| Kriter | Durum |
|---|---|
| Baseline raporu kayıtlı | ✅ bu belge |
| T3 SD ve DFU sözleşmesi test/doküman olarak sabit | ✅ `bb-config/tests/fixtures/t3/dfu_contract.md`, `main_catalog.json`, `boot_manifest.json`, `invalid/*.json` |
| Ürün kapsamı ADR'si mevcut | ✅ `docs/adr/0001-product-scope.md` (`T3Only`, güvenli varsayılan) |
| Değişiklik öncesi test durumu yeniden üretilebilir | ✅ ham loglar `docs/baseline/*.log`; komutlar §3'te birebir listelendi |

**Açık kalan eksikler** (Faz 1'e geçişi engellemez, ama izlenmeli):
1. Release binary boyutu ölçülmedi (§6).
2. `bb-imager-gui` test binary'si bu host'ta UAC elevation hatasıyla çalışmıyor (§3.2) — CI/diğer geliştirici host'larında doğrulanmalı.
3. `make`, `cargo-deny`, `cargo-packager` bu host'ta kurulu değil (§2.1).
4. Fixture'lardaki T3 katalog/boot manifest verileri **sentetik**tir, gerçek `packages.t3gemstone.org` cevabıyla karşılaştırılmadı (canlı ağ erişimi bu Faz 0 turunda kullanılmadı) — Faz 1'de gerçek veriyle çapraz doğrulanmalı.
