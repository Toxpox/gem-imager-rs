# Faz 4.5 — Katalog adaptörünün ön yüze bağlanması raporu

Planda numaralı bir faz değil. `planOptCl.md` Faz 2 çıkış kriteri ("GUI'de T3 seçilince T3 imajları
görünür") ve `instruction.md` §6 gereği. Tarih: 2026-08-01.
Host: Windows 11, Git Bash + PowerShell.
Başlangıç: `c35fef6` (Faz 4 sonu) · Bitiş: `830b3cd` · Dal: `t3-gemstone-imager`

---

## 0. Neden bu faz var

Faz 4 bittiğinde uygulama açıldığında **T3-GEM-O1 görünmüyordu** — yani forkun tek varlık sebebi
olan kart, dört fazdır seçilemez durumdaydı. Bunu kullanıcı bildirdi; hiçbir faz raporu "ana hedef
hâlâ ulaşılamaz" diye kapatmamıştı.

Yeni bir regresyon değil. Faz 2 raporu bunu yazmıştı:

> `FAZ2_REPORT.md:64` — "adaptörü ve `t3::store` yazıldı ve testli, ama **GUI'ye bağlanmadı**.
> GUI hâlâ [eski şemayı okuyor]. Bu bağlantı `instruction.md`'de ayrı numaralı bir faz değil;
> Faz 3/4 ile birlikte yapılmalı."

Faz 3 raporu aynı maddeyi tekrarladı (`FAZ3_REPORT.md:172-173`). Numaralı bir fazın parçası
olmadığı için iki fazdır kaydı yapılıp ertelendi. Bu rapor o borcu kapatıyor.

**Süreç dersi:** "ayrı numaralı bir faz değil" notu bir kapatma planı değil, sürekli ertelenme
mekanizmasıydı. Ürün hedefine doğrudan bağlı bir açık madde, plandaki faz numarasından bağımsız
olarak bir sonraki fazın çıkış kriterine yazılmalıydı.

---

## 1. Kök neden

İki ayrı arıza üst üste bindiği için sonuç sessiz bir boşluktu.

### 1.1 Ön yüz yanlış katalogu okuyordu

`config.json` içindeki `remote_configs` `beagleboard/distros/os_list.json`'a bakıyordu ve gömülü
`devices` listesinde yalnız BeagleY-AI (etiket `beagle-am67`) vardı. `packages.t3gemstone.org`
dosyada hiç geçmiyordu.

### 1.2 Doğru katalogu göstermek de yetmezdi — sessizce boşalırdı

Canlı katalog (`packages.t3gemstone.org/images/list.json`, bu oturumda indirildi) üst düzeyde
legacy şemayla **aynı şekle** sahip (`{imager:{devices}, os_list}`), ama iki alanda ayrılıyor:

| Alan | T3 kataloğu | `bb_config::config` | Sonuç |
|---|---|---|---|
| `devices[].flasher` | **yok** | zorunlu | `VecSkipError` → **tüm device'lar düşer** |
| `os_list[].init_format` | `"systemd"` | `none\|sysconf\|armbian\|cloudinit` | `VecSkipError` → **tüm image'lar düşer** |

Her iki struct da `VecSkipError` ile ayrıştırıldığı için bu **hata vermez**. Ayrıştırma başarılı
döner, liste boş gelir, kullanıcı "katalog boş" görür. `planOptCl.md` §3.2'nin "sessiz boş liste"
tuzağı tam olarak budur ve katı adaptör bunun için yazılmıştı — ama ön yüz adaptörü hiç
çağırmadığı için tuzağa arka kapıdan girilmişti.

### 1.3 Adaptörün tüketicisi yoktu

`grep -rln "t3::" --include=*.rs`, `bb-config` dışında **sıfır** dosya döndürüyordu.
`bb-config/src/t3/*` (~2.600 satır + 851 satır test) tamamen ölü kütüphaneydi. Faz 3'ün GUI
tarafındaki bütünlük bağlantısı da öyle: derleyici `bb-imager-gui/src/helpers.rs`'te
`extract_gate`, `open`, `into_image_fn` için "never used", `extract_sha256`/`downloader` için
"never read" uyarısı veriyordu. Bu uyarılar dört fazdır derleme çıktısındaydı.

---

## 2. Yapılan

### 2.1 `bb-config/src/t3/bridge.rs` (yeni)

`catalog_to_config(&ValidatedT3Catalog) -> config::Config`. Doğrulanmış canonical modeli ekranların
zaten çizdiği modele çeviriyor. Ağdan gelen belge **hiçbir zaman** doğrudan `config::Config`'e
deserialize edilmiyor; önce `raw` → `validate` → `canonical` katmanlarından geçiyor.

Bilinçli olarak çevrilmeyenler:

- **Özelleştirme.** T3 image'ının `systemd` tüketicisi GemInit `config.ini` yazıcısıdır;
  `InitFormat::Sysconf` ise BeagleBoard'un `sysconf.txt`'idir. Eşlemek, GUI'nin karta hiç okumadığı
  bir dosya yazmasına yol açardı. Köprülenen her image `InitFormat::None` raporluyor — Faz 5'in işi.
- **eMMC/DFU yeteneği.** `config::Flasher` yalnız `SdCard` ifade edebiliyor. Kartlar DFU
  profillerini canonical modelde koruyor; köprülenen görünüm bilerek SD-only — bu da M1'in SD-only
  kilometre taşıyla örtüşüyor.

İkon: `OsImage.icon` zorunlu, canonical `Image.icon` opsiyonel. URL uydurmak yerine image ikonu →
onu kabul eden kartın ikonu sırasıyla çözülüyor; hiçbiri yoksa image **loglanarak** atlanıyor,
sessizce değil. (Canlı katalogda 7/7 image ve her iki kart ikon yayınlıyor, yani bu yol pratikte
tetiklenmiyor.)

### 2.2 GUI yönlendirmesi

`bb-imager-gui/src/helpers.rs::fetch_remote_config` — uzak config URL'i T3 katalog **host**'una
aitse adaptör + bridge yolundan, değilse eski yoldan geçiyor. Host üzerinden eşleşiyor (tam URL
değil) ki bir ayna veya yol değişikliği sessizce legacy parser'a düşmesin.

Düşen/indirgenen her girdi JSON path'iyle `tracing::warn!` olarak yayınlanıyor; kapsama giren kart
ve image sayısı `tracing::info!` ile loglanıyor. Küçülen bir katalog, normal ve küçük bir listeden
ayırt edilebilir olmalı.

### 2.3 Ürün kapsamı

`ProductScope::T3AndBeagleY` — **T3-GEM-O1 ve BeagleY-AI, başka hiçbir şey.** Katalog ayrıca
etiketsiz bir "No filtering" sözde-cihazı yayınlıyor; bu, isim kontrolüyle değil, hiçbir kart
etiketi taşımadığı için kapsam dışı kalıyor.

### 2.4 `config.json`

`remote_configs` → `https://packages.t3gemstone.org/images/list.json`. Gömülü `devices` boşaltıldı:
kartlar artık kataloğun kendisinden geliyor, böylece gömülü ve uzak kayıtlar çakışmıyor.

---

## 3. Ölçülen sonuçlar

```
cargo fmt --all -- --check                             → temiz
cargo test -p bb-config --test t3_catalog              → 19 geçti
cargo test --all-features --workspace                  → 225 geçti / 26 suite
cargo clippy --all-targets --all-features --workspace  → 0 hata, 9 uyarı (hepsi mevcut:
                                                         bb-drivelist + pal/windows.rs:132)
cargo check -p bb-imager-gui --features sd             → 0 hata, 1 uyarı (mevcut: helpers.rs:601)
cargo build --release -p bb-imager-gui --features sd   → başarılı (23.4 MB)
```

Faz 4 sonunda 216 test vardı; bu fazda +9 (7 unit + 2 entegrasyon).

### Yeni testler

`bb-config/src/t3/bridge.rs` (7 unit):

- `the_legacy_parser_silently_produces_an_empty_catalog` — **bu fazın regresyon testi.** Canlı
  şekilli baytlar düz `Config` olarak ayrıştırıldığında hata vermeden boş device ve boş `os_list`
  ürettiğini kanıtlıyor. Kök nedeni koda sabitler.
- `the_bridge_exposes_the_t3_board_and_its_image`
- `t3_only_scope_hides_beagley_and_its_images` / `combined_scope_exposes_both_boards_with_their_own_images`
- `both_integrity_gates_survive_the_bridge` — archive ve extract hash/boyutu çeviriden sağ çıkıyor.
- `customization_is_not_mapped_onto_the_beagleboard_format`
- `the_bridged_config_declares_no_further_remote_configs`

`bb-config/tests/t3_catalog.rs` (2 entegrasyon, gerçek katalog fixture'ı üzerinde):

- `the_live_catalog_reaches_the_front_end_model_with_both_product_boards` — ön yüz modelinde tam
  olarak `["BeagleY-AI", "T3-GEM-O1"]`; etiketsiz sözde-cihaz kart listesine girmiyor; boş image
  listesi başarı sayılmıyor; her image kapsamdaki bir karta ait ve extract hash/boyutunu koruyor.
- `the_t3_board_has_images_in_the_front_end_model` — T3 seçimi çıkmaz sokak değil.

**Fixture tazeliği:** `bb-config/tests/fixtures/t3/main_catalog.json`, bu oturumda canlı
`packages.t3gemstone.org/images/list.json` ile indirilip karşılaştırıldı — normalize JSON olarak
**birebir aynı** (3 device, 7 image). Testler bu yüzden aynı zamanda şema kayması dedektörü.

### Platform durumu

- **Windows:** derlendi, 225 test geçti, release GUI build'i alındı.
- **Linux / macOS:** derlenmedi (bu host'ta yalnız `x86_64-pc-windows-msvc` target kurulu). Bu
  fazın dokunduğu kod platform-bağımsız, ama derlenmiş sayılmamalı.

### Gerçek uygulama koşumu

**Yapılmadı.** GUI derlendi ama çalıştırılıp T3-GEM-O1'in ekranda göründüğü gözlemlenmedi. Uçtan
uca kanıt, gerçek katalog fixture'ı üzerindeki testtir — ekran görüntüsü değil. Ekran doğrulaması
kullanıcı tarafında yapılacak; log'daki
`T3 catalog: N board(s) and M image(s) in scope` satırı bu adımın kanıtı olur.
Log yolu: `%LOCALAPPDATA%\beagleboard\imagingutility\org.beagleboard.imagingutility.log`.

**Kalıcı DB endişesi yok:** `db::Db::new()` her açılışta `tempfile::NamedTempFile` oluşturuyor,
yani SQLite kataloğu diskte kalıcı değil. Eski BeagleBoard kayıtları bir sonraki açılışta kendi
kendine yok oluyor; kullanıcının cache temizlemesi gerekmiyor.

---

## 4. Açık kalanlar

1. ~~**Faz 3'ün GUI bütünlük bağlantısı hâlâ ölü.**~~ **DÜZELTME (2026-08-01): bu madde yanlıştı.**
   Kapı Faz 3'ten beri bağlı ve çalışıyor. Kanıt ve incelemenin ortaya çıkardığı *gerçek* sorun
   için §6.
   > Özgün metin, kayıt için: *"`helpers.rs`'teki `extract_gate`, `open`, `into_image_fn` 'never
   > used' uyarıları duruyor. Bridge `extract_sha256`'yı modele taşıyor, ama GUI'nin indirme yolu bu
   > değeri `ExtractGate::Declared`'a bağlamıyor — yani indirilen imaj Faz 3'ün extract kapısından
   > geçmiyor. Faz 4'ün read-back'i yazılan baytları doğrular, fakat yanlış imajın indirilmiş
   > olmasını yakalayamaz. Sıradaki iş bu olmalı."*
2. **CLI'da kart seçimi yok.** `bb-imager-cli` katalog okumuyor; yalnız `flash sd` ve `flash dfu`
   alt komutları var. T3 orada da görünmez.
3. **Çevrimdışı ilk açılış boş.** Gömülü `devices` boşaltıldığı için ilk çalıştırmada ağ yoksa kart
   listesi boş gelir. Faz 3'ün `t3::store` last-known-good cache'i hâlâ bağlı değil — bağlanınca bu
   kapanır (`FAZ3_REPORT.md` §5'te zaten açık madde).
4. **`ProductScope` sabit kodlu.** GUI'de `T3AndBeagleY` olarak gömülü; ürün kararı değişirse ayar
   veya derleme bayrağı olmalı (`instruction.md` §5.2, ADR 0001).
5. **DFU yeteneği ön yüze taşınmıyor.** Köprülenen görünüm SD-only; `BoardCapabilities` ve
   `DfuProfile` canonical modelde duruyor ama ekranlara ulaşmıyor. Faz 7-8.
6. **Özelleştirme kapalı.** Tüm köprülenen image'lar `InitFormat::None`. Faz 5 T3 serializer'ını
   getirene kadar kullanıcı hostname/Wi-Fi/parola giremez.
7. **Marka hâlâ BeagleBoard.** `APP_NAME = "BeagleBoard Imager"`, `PACKAGE_QUALIFIER =
   ("org","beagleboard","imagingutility")`. Faz 6.

---

## 5. Commit serisi

```
830b3cd  feat(config,gui): show T3-GEM-O1 by bridging the T3 catalog to the front-end
```

Tek commit. `bridge.rs`, GUI yönlendirmesi ve `config.json` birbirinden bağımsız anlam taşımıyor:
bridge'siz yönlendirme derlenmez, yönlendirmesiz `config.json` değişikliği ise kataloğu legacy
parser'a verip **listeyi boşaltır** — yani ara commit çalışan bir durum değil.

Geri alma: commit tek başına revert edilebilir; ön yüz eski BeagleBoard kataloğuna döner.

### Değişen dosyalar

```
bb-config/Cargo.toml               tracing bağımlılığı (atlanan image'ı loglamak için)
bb-config/src/t3/bridge.rs         yeni — canonical → config::Config çevirisi + 7 test
bb-config/src/t3/mod.rs            pub mod bridge + catalog_to_config re-export
bb-config/tests/t3_catalog.rs      canlı fixture üzerinde 2 uçtan uca test
bb-imager-gui/src/helpers.rs       is_t3_catalog + fetch_remote_config
bb-imager-gui/src/message.rs       DbInitSuccess uzak config görevi yeni yola bağlandı
config.json                        remote_configs → T3 kataloğu; gömülü devices boşaltıldı
Cargo.lock                         tracing
```

### Çalışma ağacındaki ilgisiz değişiklikler

Faz 4'te `cargo fmt --all` workspace genelinde çalıştığı için üç dosyada saf biçimlendirme artığı
commit edilmeden duruyor (davranış değişikliği yok):

```
bb-helper/src/lib.rs               modül sırası
bb-helper/src/reader_progress.rs   fazladan boş satır
bb-imager-gui/src/constants.rs     satır sarma
```

Dördüncüsü (`bb-imager-gui/src/message.rs`) bu fazın commit'ine dahil oldu, çünkü aynı dosya
fonksiyonel olarak da değişti. Kalan üçü ayrı bir `style:` commit'i olmalı veya
`git checkout --` ile geri alınmalı. (Geri alma bu oturumda denendi, izin katmanı tarafından
reddedildi.)

---

## 6. Düzeltme — §4.1 yanlıştı (2026-08-01)

Faz 5 bittikten sonra §4 madde 1 "sıradaki iş" olarak incelendi. **İddia yanlış çıktı: extract
kapısı GUI indirme yoluna bağlıydı ve çalışıyordu.** Aşağıdaki düzeltme, hatanın nasıl yapıldığını
da kaydediyor; çünkü aynı tuzağa tekrar düşmek kolay.

### 6.1 Zincir aslında tamdı

```
flash()
  → SelectedImage::into_image_fn()
  → RemoteImage::into_image_fn()      helpers.rs:334
  → RemoteImage::extract_gate()       helpers.rs:279  → ExtractGate::Declared
  → OsImage::from_path / from_piped(gate)
  → OsImage::read()                   img/mod.rs:88   — her bayt sayılıp hash'leniyor
  → ExtractVerifier::settle()         EOF'ta iki kapı da kontrol ediliyor
```

Veri yolu da tamdı: `bridge.rs` `extract_sha256`'yı `Some(...)` olarak taşıyor, `os_images`
tablosunda `extract_sha256` sütunu var, `RemoteImage::new` değeri alıyor (`helpers.rs:79`). Cache
yolu ve canlı indirme yolu **aynı** gate nesnesini kullanıyor.

Kapıyı bağlayan commit `0bd8c4d — "feat(gui): name the archive hash correctly and enforce the
extracted gate"`, yani **Faz 3**. Bu rapor yazıldığında iş çoktan yapılmıştı.

### 6.2 Yanlış teşhisin sebebi: feature bayrağı

Raporun dayandığı "never used" uyarıları gerçekti ama yanıltıcıydı. `flash()`'in match kolları
`#[cfg(feature = "sd")]` ile işaretli ve **`sd`, `bb-imager-gui`'nin default feature'ı değil**
(`default = ["static"]`). Dolayısıyla:

- `cargo check -p bb-imager-gui` → `into_image_fn` çağrılmıyor → "never used".
- `cargo check -p bb-imager-gui --features sd` (Makefile'ın `_RUST_ARGS_GUI`'si) → temiz.

Bu tam olarak `instruction.md` §4 çalışma kuralı 12'nin uyardığı durum: *"Doğrulama için çıplak
`cargo check`/`cargo test` kullanma. Feature bayrakları büyük kod bloklarını kapatır ve çıplak
komutlar bunları sessizce atlayıp yanlış 'yeşil' üretir."* Burada ters yönde çalıştı — yanlış
*kırmızı* üretti ve var olmayan bir iş maddesi doğurdu.

### 6.3 Gerçekten eksik olan: test edilmemiş bir varsayım

Kapı yalnız decoder EOF bildirdiğinde `settle` oluyor. Yani tüm garanti, yazıcının *deklare edilen
bayt sayısına ulaşınca durmayıp* EOF'a kadar okumasına bağlı (`read_aligned`, `flashing/mod.rs:124`).
Bunu hiçbir test doğrulamıyordu: bu davranış bir gün regresyona uğrasa kapı **sessizce hiç
ateşlenmez**, flash başarılı görünürdü.

`bb-flasher`'a iki test eklendi (`sd::Flasher` üzerinden, dosya hedefiyle, cihaz gerekmiyor):

- `a_declared_gate_mismatch_fails_the_whole_flash` — doğru boyut/yanlış digest tüm flash'ı düşürüyor.
- `a_matching_declared_gate_flashes_to_completion` — doğru digest tamamlanıyor, yani üstteki test
  beyan edilen sebepten kırılıyor, "bu yol zaten hiç çalışmıyor" diye değil.

### 6.4 Gerçek sorun: hata sebebi kullanıcıya ulaşmıyordu

İlk test beklenmedik biçimde kırıldı ve sebebi öğreticiydi. Kapı ateşleniyor, flash **başarısız
oluyor** — ama kullanıcının gördüğü metin şuydu:

```
Unknown Error during IO. Please check logs for more information.
```

`bb-flasher-sd` decoder'ın `io::Error`'ını catch-all `Error::IoError` varyantına sarıyor; GUI de
(`main.rs:300`) `e.to_string()` ile yalnız **en dıştaki** mesajı basıyordu. Sonuç: yanlış ya da
bozuk imaj indirilmesi, dolu disk veya izin hatasıyla **birebir aynı** görünüyordu. İşe yarayacak
tek eylem — "tekrar indir" — kullanıcı için keşfedilebilir değildi.

Bütünlük hattının ön yüzde gerçekten koptuğu yer burasıydı: hata yutulmuyor, **teşhisi** yutuluyordu.
Bayt sayıları ve mismatch bilgisi hatanın içinde zaten vardı, sadece ekrana çıkmıyordu.
`format!("{e:#}")` ile kaynak zinciri basılıyor artık.

### 6.5 Commit

```
a57de08  fix(gui): report why a flash failed instead of "Unknown Error during IO"
```

`bb-flasher` 68 test geçiyor; clippy ve fmt temiz (kalan iki uyarı dokunulmayan dosyalarda ve bu
işten önce de vardı).

### 6.6 §4 madde 1'in yerine geçen açık madde

Kapı bağlı ve testli. Geriye kalan gerçek boşluk daha dar: **GUI sınırında `extract_gate()`'in
`Declared` döndürdüğünü doğrulayan bir test yok.** Kapının kendisi `bb-flasher`'da testli, veri
yolu `bb-config` ve `db` testlerinde testli, ama "köprülenmiş bir T3 imajı için GUI gerçekten
`Declared` üretiyor mu" iddiası uçtan uca hiçbir testte yer almıyor. Küçük bir test, ucuz.
