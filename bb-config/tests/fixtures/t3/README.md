# T3 Faz 0 fixture'ları

`instruction.md` §5.3 gereği sürüm kontrollü, kişisel veri içermeyen sabit örnekler.

**Faz 1 güncellemesi (2026-07-31):** `main_catalog.json` ve `boot_manifest.json` artık **sentetik değil** — canlı servisten birebir çekilmiş kayıtlardır. Bu sayede `tests/t3_catalog.rs` aynı zamanda bir şema-drift dedektörü işlevi görür: sunucu şeması değişirse testler kırılır.

## Dosyalar

- `main_catalog.json` — `https://packages.t3gemstone.org/images/list.json` birebir kaydı (2026-07-31). 3 device girdisi (biri tagsiz `No filtering` pseudo-device), 34 leaf image (17 T3 + 17 BeagleY), tek seviyeli `Pardus`/`Ubuntu`/`Debian` sub-list'leri.
- `boot_manifest.json` — `https://packages.t3gemstone.org/images/boot/t3-gem-o1/list.json` birebir kaydı. **Yalnız `files[].name` ve `files[].sha256` yayınlar**; `alt_setting`, `url`, `size` alanları yoktur. Bu yüzden DFU alt-setting eşlemesi `instruction.md` §3.2 sözleşmesinden gelen compile-time sabittir (`DfuProfile::t3_gem_o1()`), sunucu verisi değil.
- `invalid/missing_hash.json` — `extract_sha256` alanı eksik olan bir image + sağlam bir komşu.
- `invalid/bad_url_downgrade.json` — `url` alanı `http://` (HTTPS downgrade) + sağlam bir komşu.
- `invalid/orphan_tag.json` — orphan device tag, sıfır `extract_size` ve 63 karakterlik kısa hash + sağlam bir komşu.
- `dfu_contract.md` — `instruction.md` §3.2 ve §12.3'teki DFU sözleşmesinin test/doküman olarak sabitlenmiş kopyası (Faz 0 çıkış kapısı gereksinimi).

Her geçersiz fixture bilinçli olarak **sağlam bir komşu** içerir: tek bozuk girdinin tüm kataloğu zehirlememesi gerektiğini doğrular (§6.2 kısmi katalog kuralı).

## Kullanılmayan büyük ikili veri

Fixture'lar gerçek OS imajı veya boot artifact içermez. Testler küçük sentetik byte dizileri (örn. birkaç KB) ile kendi mock XZ/raw payload'larını üretmelidir.
