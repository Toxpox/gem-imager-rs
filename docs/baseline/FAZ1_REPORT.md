# Faz 1 — Canonical model ve katı T3 katalog adaptörü

Tarih: 2026-07-31. `instruction.md` §6 ve §20 raporlama şablonu.

## Amaç

`instruction.md` §6'nın tamamı: üç katmanlı katı T3 katalog adaptörü (§6.1–6.2), kart yeteneği ve hedef seçimi (§6.3), sürümlü kalıcı SQLite (§6.4) ve §6.5 test listesi. §15'teki PR 4, 5 ve 6'ya karşılık gelir.

## Canlı şema doğrulaması — Faz 0 fixture'larının değiştirilmesi

Faz 0 raporu fixture'ların sentetik olduğunu ve "Faz 1'de gerçek veriyle çapraz doğrulanmalı" diye işaretlemişti. Yapıldı; her iki servis de `HTTP 200` döndürdü ve fixture'lar **birebir canlı kayıtla** değiştirildi.

Gerçek T3 şeması `bb-config/src/config.rs`'teki BeagleBoard şemasından yapısal olarak farklı:

| Alan | Gerçek T3 kataloğu | BeagleBoard şeması |
|---|---|---|
| `devices[].emmc` | `bool` — T3-GEM-O1 `true`, BeagleY-AI `false` | yok |
| `devices[].matching_type` | `"inclusive"` / `"exclusive"` | yok |
| `devices[].flasher` | **yok** | var (`Flasher` enum) |
| `os_list[].extract_sha256` | **var**, 34/34 image'da | yok |
| `os_list[].tags` | yok (yalnız `devices`) | var |
| `init_format` | `"systemd"` | `none`/`sysconf`/`armbian`/`cloudinit` |
| Boot manifesti | yalnız `files[].name` + `files[].sha256` | — |

Ölçümler (34 leaf image): T3-GEM-O1 17, BeagleY-AI 17; dört bütünlük alanı da 34/34 mevcut; sıfır adet HTTPS-dışı URL; sıfır adet 64-hex olmayan hash; `extract_size` hiçbirinde sıfır değil.

### Faz 0 raporundaki tespitin düzeltilmesi

Faz 0 raporu "`extracted_sha256` alanı hiç yok" diyordu. Bu **`bb-config`'in Rust tipi için** doğrudur, ancak **canlı T3 kataloğu `extract_sha256` alanını yayınlıyor**. Dört kapının dördü de sunucu tarafından sağlanıyor; eksik olan yalnız Rust modeliydi. Faz 0 raporu bu düzeltmeyle güncellendi.

### Boot manifesti bulgusu (Faz 7'yi doğrudan etkiler)

Gerçek `boot/t3-gem-o1/list.json` **yalnız** şunu yayınlıyor:

```json
{ "files": [ { "name": "tiboot3.bin", "sha256": "..." }, ... ] }
```

`alt_setting`, `url`, `size`, `reset_after`, VID/PID alanları **yoktur**. Dolayısıyla DFU alt-setting eşlemesi sunucu verisi olamaz; §3.2 sözleşmesinden gelen **compile-time sabit** olarak `DfuProfile::t3_gem_o1()` içinde modellendi. Faz 7'deki resolver artifact URL'lerini manifest taban yolundan türetmek zorunda.

## Değişen sözleşme

Yeni `bb_config::t3` modülü. Mevcut `bb_config::config` (BeagleBoard) API'si **hiç değiştirilmedi** — kırılma yok.

- `t3::raw` (Katman 1) — sunucu şemasına yakın serde tipleri. Doğrulamaya konu her alan `Option`; bozuk girdi tüm belgeyi düşürmüyor, kendi tanısını üretiyor. Bilinmeyen alanlar yok sayılıyor (canlı `random` alanı dahil). **`VecSkipError` bu modülde hiç kullanılmadı.**
- `t3::validate` (Katman 2) — §6.2 invariantları; her ret JSON path taşıyan `T3Diagnostic` üretiyor, `rejected_images`/`rejected_boards` sayaçları "kaç girdi neden reddedildi" sorusunu doğrudan cevaplıyor.
- `t3::canonical` (Katman 3) — `Board`, `Image`, `ImageIntegrity`, `BoardCapabilities`, `DfuProfile`, `DfuStageSpec`, `CustomizationProfile`, `WriteMethod`, `ProductScope`.
- `t3::sha256` — `Sha256` newtype; parse sırasında tam 64 hex → `[u8; 32]`.
- `t3::store` — sürümlü kalıcı SQLite (`PRAGMA user_version`).

## Güvenlik ve veri bütünlüğü

- **Dört bütünlük kapısı ayrı alanlarda**: `archive_sha256`, `archive_size`, `extracted_sha256`, `extracted_size`. Tip düzeyinde birbirinin yerine geçemezler; SQLite'ta da dört ayrı sütun.
- **Hash parse'ı**: 64 hex dışındaki her şey reddediliyor; kısa hash sıfırla uzatılmıyor (63 karakter → `Length { actual: 63 }`).
- **HTTPS zorunlu**: `url` alanında `http://` reddediliyor.
- **`extract_size > 0`** zorunlu.
- **Orphan/eksik tag** sessizce atılmıyor; JSON path'li tanı üretiyor, image reddediliyor.
- **Boş sonuç başarı sayılmıyor**: `NoBoards` / `NoUsableImages`.
- **`emmc: true` yalnız doğrulanmış T3 profili üretiyor**: T3 olmayan kart eMMC iddia ederse SD'yi koruyor, DFU alamıyor, tanı üretiyor.
- **`artifact_name` ≠ `alt_setting`**: ayrı alanlar; `tiboot3.bin` → `bootloader` farkı testle sabitlendi.
- **Bilinmeyen `init_format`**: image flash edilebilir kalıyor, özelleştirme kapanıyor (ne yazacağımızı bilmiyorsak yazmıyoruz).
- **Store**: gelecek şema sürümü reddediliyor; bozuk hash sütunu `Corrupt` hatası veriyor; boyutlar `as` ile değil `try_from` ile dönüştürülüyor.
- **`DROP and recreate` yok**: migration `PRAGMA user_version` üzerinden, her biri kendi transaction'ında.

## Testler

- `cargo test -p bb-config --test t3_catalog` → **17 passed; 0 failed**
- `cargo test -p bb-config --test t3_store` → **13 passed; 0 failed**
- `cargo fmt -p bb-config -- --check` → temiz
- `cargo clippy -p bb-config --all-targets --all-features` → **0 uyarı, 0 hata**

Workspace regresyonu (Faz 0 baseline komutlarıyla birebir aynı kapsam): `check_common`, `check_cli`, `check_gui`, `test_common`, `test_cli` → hepsi `EXIT=0`. Faz 0'da kayıtlı `bb-imager-gui` UAC hatası host sorunudur, değişiklikle ilgisizdir.

### §6.5 karşılama tablosu

| §6.5 maddesi | Durum |
|---|---|
| Gerçek fixture'daki tüm T3 image'lar doğru bağlanır | ✅ `every_t3_image_in_the_live_catalog_binds_correctly` (17 T3 image) |
| BeagleY image'lar doğru bağlanır ve SD-only kalır | ✅ `the_t3_board_gets_a_verified_dfu_profile_and_beagley_stays_sd_only` |
| Zorunlu alan hataları JSON path ile döner | ✅ `missing_required_field_is_reported_with_its_json_path` (`os_list[0]`) |
| Boş katalog hata verir | ✅ `an_empty_catalog_is_an_error_not_a_success` |
| Bilinmeyen ek alan pozitif testten geçer | ✅ `unknown_future_fields_are_ignored_for_forward_compatibility` |
| Capability round-trip SQLite testi | ✅ `the_dfu_profile_survives_a_round_trip_with_its_stage_order` |
| Migration önceki şemadan veri kaybetmeden ilerler | ⚠️ kısmen — aşağıya bakın |

## Bilinen sınırlamalar

1. **Migration veri-koruma testi tam yazılamadı.** Şu an yalnız v1 şeması var; "v1'den v2'ye veri kaybetmeden geçiş" testi ancak gerçek bir v2 var olduğunda yazılabilir. Sırf test için sahte bir v2 eklemek testin kendisini test etmek olurdu. Bunun yerine çerçevenin doğrulanabilir kısımları test edildi: taze DB doğru sürüme geliyor, mevcut DB'yi yeniden açmak satırları koruyor (DROP-recreate değil), gelecek sürüm kontrollü reddediliyor, bozuk dosya ve bozuk satır tipli hata veriyor.

2. **`t3::store` GUI'ye bağlanmadı.** `bb-imager-gui/src/db/mod.rs` hâlâ eski `tempfile::NamedTempFile` tabanlı DB'yi kullanıyor. Bilinçli kapsam kararı: §15'te PR 6 ile PR 17 ayrı; GUI'yi şimdi yeniden bağlamak Faz 8 işini öne çekmek ve §4.10'u ihlal etmek olurdu. Eski DB modülü silinmedi, dokunulmadı.

3. **Boot manifest resolver yazılmadı.** §12.5 fail-closed resolver Faz 7 / PR 15 işi.

4. **`desktop_variant` bir sezgiye dayanıyor.** T3 kataloğunda açık desktop bayrağı yok; image adında "desktop" geçmesine bakılıyor. Yalnız VNC alanlarının *gösterilip gösterilmeyeceğini* kapılıyor. Faz 5'te gerçek `gem-first-boot` consumer'ına karşı doğrulanmalı.

5. **`matching_type` modellendi ama uygulanmadı.** Filtreleme davranışı UI katmanına ait (Faz 8).

6. **`ProductScope` varsayılanı `T3Only`** (ADR 0001). BeagleY kartları modelde kalıyor, yalnız `in_product_scope: false` işaretleniyor — §3.1 gereği.

## Gerçek veriden çıkan sürpriz bulgu

Canlı katalog **4 URL'yi iki kez yayınlıyor** (aynı Ubuntu imajları hem üst seviyede hem "Ubuntu Images" sub-list'i içinde; 34 leaf, 30 tekil URL). İlk şema taslağımda `images.url` üzerinde `UNIQUE` kısıtı vardı ve testler bunu yakaladı. Kısıt kaldırıldı: bir image satırının kimliği `(url, image_group)` çiftidir. Dedupe **yapılmıyor** — ikisi de kullanıcının seçebileceği gerçek listelemeler. `the_same_url_may_legitimately_appear_under_more_than_one_group` testiyle sabitlendi.

## Platform durumu

- **Windows**: derlendi ve test edildi (bu host).
- **Linux**: bu turda çalıştırılmadı — `bb-config` platform-bağımsız saf Rust; risk düşük ama CI'da doğrulanmalı.
- **macOS**: çalıştırılmadı.

## Geri alma

`bb-config` içinde ek: `src/t3/`, `tests/t3_catalog.rs`, `tests/t3_store.rs`, `tests/fixtures/t3/`. `src/lib.rs`'te tek satır (`pub mod t3;`), `Cargo.toml`'da `serde_json` deps'e taşındı + `const-hex`'e `alloc` + `tempfile` dev-dep. Geri almak için `t3` modülünü ve testleri silmek, `lib.rs` satırını kaldırmak ve `Cargo.toml`'u eski haline döndürmek yeterli; mevcut hiçbir kod yolu bu modüle bağlı değil.
