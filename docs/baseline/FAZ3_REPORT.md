# Faz 3 — İndirme, bütünlük ve kalıcı cache raporu

`instruction.md` §8 gereği. Tarih: 2026-07-31. Host: Windows 11, Git Bash + PowerShell.
Başlangıç: `e5efbdf` (Faz 2 sonu) · Bitiş: `0bd8c4d`

---

## 0. Faz 2 doğrulaması

| Kriter | Durum | Kanıt |
|---|---|---|
| Budama tamamlandı, workspace 10 crate | ✅ | `docs/baseline/FAZ2_REPORT.md` |
| Faz 1 canonical model testleri yeşil | ✅ | `cargo test -p bb-config` faz başında 63 test |
| Faz 0 fixture'ları yerinde | ✅ | `bb-config/tests/fixtures/t3/*` |

Faz 2'nin devrettiği 6 açık maddeden **1'i bu fazda kapandı** (boot manifesti
artık ayrıştırılıyor ve fail-closed doğrulanıyor). Kalanların durumu §5'te.

---

## 1. §8.1 — Zorunlu veri hattı

Dört değer artık dört ayrı sert kapı ve **hiçbiri diğerinin yerine geçmiyor**:

| Kapı | Nerede uygulanıyor | Tip |
|---|---|---|
| `archive_size` | `bb-downloader` akış döngüsü | `ArchiveIntegrity::size` |
| `archive_sha256` | `bb-downloader` akış döngüsü | `ArchiveIntegrity::sha256` |
| `extracted_size` | `bb-flasher::img` decoder'ı | `ExtractedIntegrity::size` |
| `extracted_sha256` | `bb-flasher::img` decoder'ı | `ExtractedIntegrity::sha256` |

Extracted kapısı **okuyucunun içinde** çalışıyor: sayım ve hash, yazıcıya
verilen baytların tam üzerinde yapılıyor, EOF'ta değerlendiriliyor. Böylece
kaynak ister cache'teki arşiv, ister hâlâ inen bir akış olsun aynı kapı geçerli.
Beklenenden uzun akış, tüm imajı açmayı beklemeden, kanıtlanır olur olmaz
reddediliyor.

`ExtractGate` bir `Option` değil, **zorunlu constructor argümanı**. Doğrulama
yapılmayan iki durum adıyla yazılıyor ve tek `rg` ile bulunabiliyor:

- `ExtractGate::LocalFile` — kullanıcının kendi seçtiği dosya; kıyaslanacak bir
  yayınlanmış özet yok.
- `ExtractGate::UndeclaredLegacyCatalog` — extracted-digest sözleşmesinden
  önceki katalog girdisi.

İkisi de "verified" olarak sunulmaz (§4.9).

### Düzeltilen adlandırma yalanı

`plan.md`'nin işaret ettiği kusur doğrulandı ve kapatıldı: `RemoteImage`'ın
`extract_sha256` adlı alanına çağrı yerinde `image_download_sha256` veriliyordu.
Alan artık `archive_sha256`; extracted özet ve boyut ayrı alanlarda taşınıyor.

---

## 2. §8.2 — Downloader kuralları

`TransportPolicy` tek yerde toplanmış politika; çağrı yerinin unutabileceği
kural kural değildir.

| Kural | Uygulama |
|---|---|
| Yalnız 2xx | `send()` başarısız statüde `HttpStatus` döndürür |
| Connect/idle/total timeout | 10 sn / 30 sn / metadata 60 sn, akış 6 sa |
| Redirect sınırı | 5 (varsayılan), aşımda `TooManyRedirects` |
| HTTPS → HTTP downgrade | `RedirectRefusal::Downgrade` ile reddedilir |
| HTTPS zorunluluğu | Varsayılan politikada plaintext URL istek gönderilmeden reddedilir |
| Maksimum body | Metadata 8 MiB; akışta yayınlanan `archive_size`, yoksa 32 GiB |
| `Content-Length` | Bütünlük kanıtı olarak **kullanılmıyor** |
| Yarım indirme final adda kalmaz | `WriterFileStream::persist` scratch + fsync + rename |
| Eşzamanlı aynı hash | Digest anahtarlı single-flight; kaybeden taraf cache'i replay eder |
| Cache kimliği | Dosya adı değil hash; hash'siz varlıklar (ikonlar) ayrı URL-adresli ad alanında |

`DownloadError` taşıma hatasını bütünlük hatasından ayırıyor
(`is_integrity_failure`). Bu ayrım §8.3'ün "çevrimdışıyız, eski katalog
gösteriyoruz" durumu için gerekli: taşıma hatasında eski kopyaya düşülebilir,
bütünlük hatasında **asla**.

`io::Error`'a `From` dönüşümü olduğu için mevcut çağrı yerleri değişmeden
derleniyor.

---

## 3. §8.3 — Last-known-good

**Katalog:** `T3CatalogStore` (Faz 1) zaten kalıcı ve sürümlü. Bu fazda şema v2
geldi: `catalog_provenance`'a `etag`/`last_modified`, ayrıca `fetched_at`
okunabilir hâle geldi. Migration `ALTER TABLE ... ADD COLUMN` kullanıyor —
diskteki katalog yükseltmede korunuyor; last-known-good cache'in bütün amacı bu.
Test: `a_v1_database_is_migrated_forward_with_its_rows_intact`.

Yalnız `validate`'ten geçmiş bir katalog `save_with_validators`'a verilebiliyor,
dolayısıyla "saklanan" ile "doğrulanmış" ayrışamıyor: ayrıştırılamayan bir
yenileme bu metoda hiç ulaşmıyor ve eski kopyayı **düşüremiyor**.

**Boot manifesti:** `bb-config::t3::boot_manifest` eklendi.

- Gerekli artifact listesi manifestten değil, doğrulanmış aşama sözleşmesinden
  (`DfuProfile::t3_gem_o1`) geliyor. Sunucu bir aşamayı düşürürse sonuç
  "daha kısa boot zinciri" değil, **ret**.
- Bozuk doküman, boş `files`, çözülemeyen hash, aynı adın iki farklı hash'le
  yayınlanması → hepsi ret. Bilinmeyen **fazladan** artifact yok sayılıyor
  (bugün çalışan kartı gelecekteki bir aşama yüzünden kilitlememek için).
- `VerifiedBootManifest` yalnızca bu kontrolden geçerek üretilebiliyor;
  eksik bir manifestten DFU başlatmanın yolu tip düzeyinde kapalı.
- `save_boot_manifest`/`load_boot_manifest` last-known-good kopyayı taşıyor;
  saklanan manifest güncel aşama sözleşmesini karşılamıyorsa **hata** dönüyor,
  eksik manifest dönmüyor.

---

## 4. §8.4 test matrisi

| Senaryo | Test |
|---|---|
| Doğru archive + yanlış extracted hash | `a_declared_gate_fails_when_the_extracted_sha256_differs` |
| Truncated XZ | `a_truncated_xz_archive_fails_to_decode` |
| Trailing garbage politikası | `trailing_garbage_after_an_xz_stream_is_refused` |
| Beklenenden kısa extracted veri | `a_declared_gate_fails_when_the_extracted_stream_is_shorter_than_declared` |
| Beklenenden uzun extracted veri | `a_declared_gate_fails_when_the_extracted_stream_is_longer_than_declared` |
| Beklenenden kısa/uzun archive | `a_body_shorter_...`, `a_body_longer_than_the_declared_archive_size_is_refused` |
| 404 / 500 | `a_404_body_is_never_mistaken_for_content`, `a_500_is_not_a_download` |
| Redirect sınırı | `a_redirect_chain_longer_than_the_limit_is_refused` |
| Redirect downgrade | `a_redirect_that_drops_https_is_refused` (+ 3 komşu birim testi) |
| İptal sonrası partial cache | `a_cancelled_download_leaves_no_partial_cache_entry` |
| Aynı hash için iki eşzamanlı indirme | `two_concurrent_downloads_of_the_same_hash_hit_the_network_once` |
| Unicode cache yolu | `a_unicode_cache_path_works` |
| Bozuk yeni katalogda fallback | `save_with_validators` sözleşmesi + `a_saved_catalog_survives_reopening_the_database` |
| Doğrulanmış manifest yoksa DFU yok | `a_board_with_no_stored_manifest_yields_nothing_to_start_dfu_with`, `a_stored_manifest_that_no_longer_covers_the_stage_contract_is_refused` |

**404 testi hakkında:** hata sayfasının kendi hash'i beklenen hash olarak
veriliyor; yani testi yalnız statü kapısı geçirebiliyor. Kapının gerçekten
gerekli olduğunun kanıtı bu.

### Ölçülen sonuçlar

```
cargo test --all-targets --all-features --workspace \
  --exclude bb-flasher --exclude bb-imager-gui --exclude bb-imager-cli   139 passed
cargo test --all-targets -p bb-flasher -F dfu,static,piped_image,sd       33 passed
cargo test --all-targets -p bb-imager-cli --features dfu                  26 passed
cargo test --doc -p bb-flasher -F sd                                       1 passed
```

Crate kırılımı: `bb-config` 81, `bb-downloader` 30, `bb-helper` 15.
Faz başındaki `bb-config` 63 testinin tamamı hâlâ geçiyor.

Clippy (Makefile'ın feature setleriyle): yalnız **iki** uyarı kaldı, ikisi de
bu fazın dokunmadığı yerlerde ve bu fazdan önce de vardı —
`bb-flasher-sd/src/pal/windows.rs:132` ve `bb-imager-gui/src/helpers.rs:601`
(`notify-rust` kapalıyken kullanılmayan `body`).

rustfmt yalnız bu fazın dokunduğu dosyalara uygulandı (§4.2: ilgisiz dosyaları
biçimlendirme; `cargo fmt --all` hâlâ Faz 2'de raporlanan pre-existing farkı
üretiyor).

---

## 5. Açık kalanlar

Bu fazın kapsamında **değil**, ama izlenmeli:

1. **Koşullu yenileme (`If-None-Match`) henüz gönderilmiyor.** ETag ve
   Last-Modified saklanıyor ve okunabiliyor; onları kullanan istek yolu
   yazılmadı. Bugünkü davranış: yenileme her seferinde tam gövde indiriyor.
   Doğrulukta kayıp yok, bant genişliğinde var.
2. **UI'da "çevrimdışı / eski katalog" göstergesi yok.** Altyapı hazır
   (`stored_fetched_at`, `DownloadError::is_integrity_failure`), gösterim Faz 6/8
   işi.
3. **GUI kataloğu hâlâ `tempfile::NamedTempFile` üzerinde** (`db/mod.rs`).
   Kalıcı last-known-good store kütüphanede (`bb-config::t3::store`) hazır ve
   testli, ancak GUI henüz canonical modele bağlanmadı — Faz 2 raporunun 1.
   maddesi. Bu bağlama yapılmadan GUI çevrimdışı açılışta hâlâ katalog görmez.
4. **`bb-imager-gui` test binary'si bu Windows host'ta çalışmıyor** (Faz 0'dan
   beri süren UAC hatası: `could not execute process ... (never executed)`).
   GUI kodu derleniyor ve clippy'den geçiyor; GUI testleri bu makinede
   koşturulamadı. Başka bir ortamda doğrulanmalı.
5. **`make`, `cargo-deny`, `cargo-packager` bu host'ta yok.** Doğrulama,
   Makefile hedeflerinin birebir feature setleri elle verilerek yapıldı.
6. **`cargo fmt --all -- --check` kırık** (13 dosya, pre-existing). Ayrı
   `style:` PR'ı gerekiyor.
7. **ADR 0001 `T3Only`**; BeagleY-AI kapsam kararı hâlâ teyitsiz (Faz 8 öncesi).

---

## 6. Commit serisi

```
c8686a2  feat(helper): publish stream files atomically
01d1cbb  feat(downloader): enforce transport policy and gate archive integrity
12c0b68  feat(flasher): gate extracted size and sha256 in the decode path
3b9bfe1  feat(config): parse the boot manifest strictly and cache it last-known-good
0bd8c4d  feat(gui): name the archive hash correctly and enforce the extracted gate
```

Her commit ayrı bir teknik amaç taşıyor ve bağımsız geri alınabilir. `Cargo.lock`
downloader commit'inde güncellendi (`thiserror`, `serde_json`, tokio `sync`/`rt`).
