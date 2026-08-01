# Faz 5 — Güvenli T3 GemInit özelleştirmesi raporu

`instruction.md` §10 ve `planOptCl.md` Faz 5. Tarih: 2026-08-01.
Host: Windows 11, Git Bash + PowerShell.
Başlangıç: `113f7bd` (Faz 4.5 sonu) · Bitiş: `9e7a372` · Dal: `t3-gemstone-imager`

---

## 0. Neden bu faz var

Faz 4.5 sonunda GUI T3 imajlarını listeliyordu ama **özelleştirme ekranı erişilemezdi**:
`bb-config/src/t3/bridge.rs` her T3 imajını `InitFormat::None`'a bağlıyordu, çünkü yazılacak
dosyayı üretecek serializer henüz yoktu. Kullanıcı imaj seçip doğrudan hedef seçimine geçiyordu.

Bu, kasıtlı bir bekletmeydi. `config.ini` sıradan bir yapılandırma dosyası değil: T3 SDK'sındaki
`gem-first-boot` bu dosyayı **`source`** ediyor. Yani dosyadaki her satır kartın ilk açılışında
**root olarak çalışan shell kodudur**. Referans uygulama (`gem-imager`) dosyayı QML string
birleştirmesiyle kuruyor:

```qml
addGemInit("hostname="+fieldHostname.text)          // OptionsPopup.qml:1060 — tırnak bile yok
addGemInit("wifiname='"+fieldWifiSSID.text+"'")     // OptionsPopup.qml:1188
```

`$(...)`, backtick veya newline içeren bir SSID/hostname bu yolla root olarak komut çalıştırır.
Bu fazın işi bu davranışı Rust'a taşımak **değil**, aynı sonucu üreten ama bu sınıf hatayı
*temsil edilemez* kılan bir yol kurmaktı.

---

## 1. Yapılan

### 1.1 `bb-flasher/src/t3_gem_init/` (yeni modül)

`planOptCl.md` §4.4 dosyayı `bb-flasher/src/flasher/sd/t3_gem_init.rs` olarak öngörüyordu.
**Sapma:** modül üst seviyeye (`bb-flasher/src/t3_gem_init/`) ve ayrı bir `t3_gem_init` feature'ına
alındı. Gerekçe: aynı `config.ini` Faz 7/8'de DFU/eMMC yolunda staging imajına da yazılacak; `sd`
feature'ının altına gömülü kalsaydı DFU tarafı SD backend'ini zorunlu çekerdi. `sd` bu feature'ı
zaten ima ediyor, dolayısıyla mevcut build'ler değişmiyor.

Dört dosya:

| Dosya | Sorumluluk |
|---|---|
| `shell.rs` | POSIX tek-tırnak literal serializer. Değerlerin dosyaya girdiği **tek** nokta. |
| `secret.rs` | `Debug` çıktısında redakte olan, drop'ta sıfırlanan `Secret` tipi. |
| `crypt.rs` | SHA-512 crypt, WPA PBKDF2, VNC DES. Başka türev yok. |
| `mod.rs` | Anahtar whitelist'i, doğrulanmış değer tipleri, dosya üretimi. |

**Anahtar whitelist'i tip sistemine gömüldü.** `enum Key` private; hiçbir public API anahtar adı
kabul etmiyor. Yani "kullanıcı anahtar adı sağlayamaz" bir kod inceleme notu değil, derleyici
garantisi. §10.1'de yasaklanan anahtarlar (`cryptsetup`, `diskpasswd`, `writeimagetommc`, gadget
toggle'ları, SSH) bu enum'da **yok** — onları yazacak bir yol bulunmuyor.

**Serialize edilen alanlar** (§10.1 tablosu, `gem-first-boot`'un gerçekten okuduğu küme):
`firstboot`, `hostname`, `userpasswd`, `wifiname`, `wifipasswd`, `wificountry`, `timezone`,
`keyboardlayout`, ve yalnız desktop imajlarında `vnc` / `vncpassword`.

**Doğrulama sınırda yapılıyor** (parse, don't validate):

| Tip | Kural |
|---|---|
| `Hostname` | RFC 1123: etiket ≤63, toplam ≤253, alnum + `-`, baş/son tire yok |
| `WifiCountry` | Tam iki ASCII harf; küçük harf büyütülüyor (yazım alışkanlığı, farklı değer değil) |
| `Ssid` | 1–32 **bayt** (karakter değil — Türkçe SSID sınıra daha erken varır) |
| `Timezone` | `chrono_tz` IANA veritabanı — GUI'nin seçicisini besleyen aynı liste |
| `KeyboardLayout` | `KEYBOARD_LAYOUTS` derleme zamanı listesi, binary search |

**CR/LF/NUL reddediliyor.** Tırnaklama bir newline'ı güvenli kılamaz: `gem-first-boot` dosyayı
satır bazlı `grep`/`sed` ile de işliyor, dolayısıyla satır yapısını bozan bir değer komut
çalıştıramasa bile dosyayı bozar. Hata tüm dosyayı düşürüyor, satırı atlamıyor.

**`firstboot=1` her zaman ilk satır.** Consumer'ın her şeyden önce test ettiği guard bu; onsuz
dosya etkisiz.

### 1.2 Parolalar — kopyalanmıyor, türetiliyor

| Alan | Türev | Kaynak |
|---|---|---|
| `userpasswd` | SHA-512 crypt (`$6$`), 5000 tur | Salt **`getrandom`** (OS CSPRNG) |
| `wifipasswd` | PBKDF2-HMAC-SHA1, 4096 tur, 32 bayt | SSID salt olarak |
| `wifipasswd` (alternatif) | 64 hex PSK doğrulanıp doğrudan kabul | — |
| `vncpassword` | Klasik VNC DES (bit-reversed sabit anahtar) | — |

`gem-imager` crypt salt'ı için Mersenne Twister besliyor; bu bir CSPRNG **değil** ve kopyalanmadı.

Wi-Fi passphrase'i karta hiç ulaşmıyor: 8–63 bayt ise PBKDF2'den geçip PSK olarak yazılıyor.
Tam 64 hex ise zaten PSK'dır ve olduğu gibi alınıyor — üzerine tekrar PBKDF2 çalıştırmak ağa
uymayan bir anahtar üretirdi. Bu ikisi arasındaki seçim uzunlukla yapılıyor, tıpkı her WPA
supplicant'ının yaptığı gibi. Başka bir uzunluk ne passphrase ne PSK'dır ve **tahmin edilmiyor**,
hata veriliyor.

VNC parolası 8 baytı aşarsa **hata**, sessiz truncation değil (§10.3). Aksi hâlde kullanıcıya
oturumu açmayan bir parola söylenmiş olurdu.

**Sır tipi.** `Secret`'ın `Debug`'ı sabit bir placeholder yazıyor ve buffer drop'ta sıfırlanıyor.
Bu stil tercihi değil: GUI state'i `Debug` türetiyor, `tracing` yapılandırılmış alanları `Debug` ile
render ediyor, panic kapsamdaki her değerin `Debug`'ını basıyor. Düz `String` üçünden de sızardı.
Üretilen dosya da `Zeroizing<Vec<u8>>`.

### 1.3 Test vektörleri — hiçbiri bu koddan gelmiyor

`instruction.md` §10.5 bağımsız doğrulama istiyor. "Kendi ürettiğimiz değere eşit" bir test test
değildir, o yüzden her ilkel dışarıdan sabitlendi:

| Test | Çapa |
|---|---|
| SHA-512 crypt | Drepper spesifikasyonunun `saltstring` vektörü + `sha512_check` ile bağımsız verify |
| WPA PBKDF2 | Üç yayımlanmış WPA vektörü; Python `hashlib.pbkdf2_hmac` ile ayrıca doğrulandı |
| DES ilkeli | FIPS 46-3 all-zeros known-answer test (`8ca64de9c1b123a7`) |
| VNC değeri | Bağımsız textbook-DES uygulamasıyla çapraz kontrol |

**Süreç notu — düzeltilen üç yanlış beklenti.** İlk yazımda üç literal ezberden yazılmıştı ve
üçü de yanlıştı: üçüncü WPA vektörü, VNC `"1234"` çıktısı ve injection testinin bir assertion'ı.
Test kırmızı verince beklentiler *koda uydurulmadı*; bağımsız hesapla doğrulandı:

- PBKDF2 üçüncü vektörü Python ile hesaplandı → `4fd16ee2…`; Rust çıktısı zaten buydu, literal
  yanlıştı.
- VNC için `gem-imager`'ın `src/dependencies/crypt/des.cpp` tablolarının standart DES olduğu
  doğrulandı (satır 28 standart IP, satır 51 standart PC-1), sonra bağımsız bir textbook-DES
  yazılıp NIST KAT ile kalibre edildi → `ee5b0e48c8fe9771`. Rust çıktısı buna eşit.
- Injection testindeki `!out.contains("EVIL=1;")` assertion'ı **kavramsal olarak yanlıştı**:
  payload literal'in *içinde* elbette görünür — tırnaklama onu silmez, etkisizleştirir. Doğru
  assertion "yeni anahtar/satır oluşmadı" ve "değer aynen geri okunuyor".

⚠️ VNC değeri **gerçek donanımda çalışan bir VNC sunucusuna karşı doğrulanmadı.** Kod, referansın
algoritmasını birebir üretiyor; sunucunun kabul ettiği donanım kabul matrisine ait bir iddiadır.

### 1.4 Yazma ve geri okuma (§10.4)

`bb-flasher-sd`'ye `ContentType::VerifiedData` eklendi: dosya yazılıyor, FAT unmount ediliyor,
**partition sıfırdan tekrar açılıp** dosya geri okunuyor ve bayt bayt karşılaştırılıyor.
Yazarın elindeki cache'ten değil karttan okunuyor — kartın okuyabildiği dosya budur.

Karşılaştırma düz eşitlik, diff değil: buffer parola hash'i taşıyabilir, dolayısıyla içeriğinin
hiçbir parçası hata mesajına veya loga düşmemeli. `CustomizationReadBackMismatch` yalnız dosya
adını taşıyor.

**Yan bulgu — gizli hata düzeltildi.** `boot_partition()` partition tablosunu stream'in bulunduğu
konumdan okuyordu. Tek açılışta sorun çıkmıyordu; ikinci açılış (geri okuma) bozuk tablo raporladı.
Artık önce `rewind()` yapılıyor. Bu, Faz 5 olmasa fark edilmeyecek bir hataydı.

### 1.5 Katalog ve GUI

`InitFormat`'a `GemInit` ve `GemInitDesktop` eklendi (SQLite kodları 5 ve 6). `Sysconf`'tan ayrı
tutuldu — o BeagleBoard'ın `sysconf.txt`'i, farklı consumer ve farklı anahtar kümesi. Birini
diğerine eşlemek uygulamanın kartın hiç okumadığı bir dosyayı yazması demekti.

Desktop, bayrak değil **ayrı varyant**: ön yüz özelleştirme ekranını bu değere göre seçiyor ve
desktop ekranı farklı bir alan kümesi sunuyor. Desktop bilgisi canonical modelden geliyor
(`CustomizationProfile.desktop_variant`), ön yüzde yeniden türetilmiyor.

GUI tarafında:

- `ui/configuration.rs`: T3 paneli. Yalnız consumer'ın okuduğu alanlar var.
- `helpers.rs`: `FlashingCustomization::T3GemInit { config, desktop }`.
- `persistance.rs`: `T3GemInitCustomization` düzenleme buffer'ı + `build()` doğrulaması.

**NEXT, serializer'ın tüm formu kabul etmesine bağlı.** Alan alan doğrulama yerine bütün form
birlikte doğrulanıyor; böylece kullanıcının bu ekranı geçip, artık göremediği bir değer yüzünden
flash sırasında hata alması mümkün değil. Geçerli olmayan formda ilk hata NEXT'in yanında
gösteriliyor.

**`sd_customization()` artık `Result` döndürüyor.** Önceden "özelleştirme yok"a düşmek mümkündü;
kullanıcının ayarladığı first-boot dosyası olmayan bir kart *bozulmuş* değil **yanlış** sonuçtur
ve ancak boot'tan sonra fark edilir (çalışma kuralı 5).

**UI'nin sakladığı iki gerçek artık açıkça yazılı:**
1. VNC parolasının 8 karakter sınırı protokolün, uygulamanın tercihi değil.
2. Kartın first-boot script'i VNC parolasını boot bölümünden **temizlemiyor** — sır kartta kalıyor.
   Bu bir SDK hatası (§3'te izleniyor); imager onu gizlemiyor.

**Hiçbir parola persist edilmiyor.** `config.json`'daki her sır alanı `#[serde(skip)]`. Yeniden
açılışta ağ adı geri geliyor, passphrase gelmiyor. Alternatifi kullanıcının config dizininde düz
metin Wi-Fi anahtarıydı.

---

## 2. Ölçülen sonuçlar

### Testler

Doğrulama `bb-imager-rs/CLAUDE.md` ve çalışma kuralı 12 gereği Makefile reçeteleriyle yapıldı.
`make` bu hostta PATH'te yok, bu yüzden reçeteler birebir `cargo clippy`/`cargo test` olarak
elle çalıştırıldı.

| Kapsam | Komut | Sonuç |
|---|---|---|
| Ortak workspace | `cargo test --all-targets --all-features --workspace --exclude bb-flasher --exclude bb-imager-gui --exclude bb-imager-cli` | **169 passed** |
| `bb-flasher` | `cargo test --all-targets -p bb-flasher -F dfu,static,piped_image,sd` | **66 passed** |
| CLI | `cargo test --all-targets -p bb-imager-cli --features dfu` | **26 passed** |
| GUI | `cargo test --all-targets -p bb-imager-gui --features sd,updater,pre-release` | **57 passed** (aşağıdaki nota bakınız) |

Clippy dört reçetede de temiz. Kalan iki uyarı Faz 5 öncesinden var ve dokunulmayan dosyalarda:
`bb-flasher-sd/src/pal/windows.rs:132` ve `bb-imager-gui/src/helpers.rs:650` (`notify-rust`
kapalıyken kullanılmayan `body`).

`cargo fmt --all --check` temiz.

#### Yeni testler (45 adet)

**`shell.rs` (5):** 13 metakarakter payload'ı literal içinde kalıyor · tek tırnak `'\''` ile
ekleniyor, düşürülmüyor · `x'; id; echo '` break-out denemesi · Unicode değişmeden geçiyor ·
CR/LF/NUL reddi.

**`secret.rs` (2):** `Debug` ve `{:#?}` plaintext içermiyor; `Debug` türeten bir struct'ın içinde
tutulduğunda da sızmıyor · uzunluk değeri açığa çıkarmadan okunabiliyor.

**`crypt.rs` (8):** crypt vektörü + bağımsız verify + yanlış parolanın doğrulanmaması · salt'ın
OS CSPRNG'den geldiği ve tekrarlanmadığı, alfabesi ve uzunluğu · üç WPA vektörü · Türkçe SSID ·
64 hex PSK pozitif/negatif (63, 65, hex olmayan) · VNC çapraz doğrulanmış vektör · FIPS DES KAT ·
9 baytlık VNC parolasının reddi ve sınırın bayt cinsinden olduğu (`çççç` geçer, `ççççç` geçmez).

**`mod.rs` (18):** boş yapılandırma da geçerli guard dosyası · `firstboot` her zaman ilk satır ·
tüm alanların beklenen satıra düşmesi ve passphrase'in dosyada olmaması · **desteklenmeyen 8
anahtarın hiç görünememesi** · injection payload'ının yeni anahtar/satır üretememesi · newline'ın
tüm dosyayı düşürmesi · RFC 1123 (6 pozitif, 8 negatif) · country code · SSID bayt sınırı ·
timezone/keymap yalnız sunulan listeden (`../../etc/localtime`, `tr; id` reddi) · liste sıralı ·
Wi-Fi uzunluk→türev seçimi · boş parola reddi · parolanın yalnız crypt hash olarak yazılması ·
VNC'nin yalnız açıkken yazılması · **izole shell round-trip (2 test)**.

Round-trip testi §10.5'in istediği doğrulama. Gerçek shell spawn etmek testi host'a ve "okuduğunu
asla çalıştırmamasına" bağımlı kılardı; bunun yerine `source`'un yaptığını yapan bir parser test
içinde yeniden yazıldı. Yalnız `key=value` ve `key='literal'` (+ `'\''`) anlıyor, başka her şey
parse hatası. Serializer bir gün *çalıştırılabilecek* bir şey üretse bu parse edilemezdi.
Kullanıcının yazdığı her değer bayt bayt geri geliyor.

**`bb-flasher-sd` (4):** FAT'a yazma + geri okuma (gerçek MBR + FAT32 imajı üzerinde) · kartta
olmayan dosyanın doğrulanamaması · yazma sonrası tek bayt flip'in mismatch vermesi · hata
mesajının dosya içeriğinden hiçbir şey alıntılamaması.

**`bb-imager-gui` (6):** hiçbir sırrın `config.json` metnine girmemesi (üç sır ayrı ayrı aranıyor)
· yeniden yüklemede ayarların gelip parolaların gelmemesi · tam formun build olması, bozuk
hostname'in olmaması · desktop olmayan imajda VNC'nin düşmesi · 9 baytlık VNC parolasının formu
düşürmesi · GUI keymap listesiyle serializer listesinin birebir aynı olması.

**`bb-config` (2):** T3 imajlarının GemInit ailesine düşmesi ve asla Sysconf/CloudInit olmaması ·
VNC'nin yalnız desktop imajlarında sunulması.

### Platform durumu

⚠️ **GUI test binary'si Windows'ta yükseltme (elevation) istiyor** — `os error 740`. Sebep
`bb-imager-gui/assets/packages/windows/gui.exe.manifest` içindeki
`requestedExecutionLevel level="requireAdministrator"`; `build.rs` bunu test binary'sine de gömüyor.
Bu Faz 0'dan beri böyle (`baseline_test_gui.log:134`), yani **GUI unit testleri bu hostta hiç
çalışmamıştı.**

Bu fazda testler manifest geçici olarak `asInvoker` yapılıp çalıştırıldı, sonra dosya yedekten
geri yüklendi. `git status` manifest'i değişmemiş gösteriyor. **Yukarıdaki 57 sonuç gerçekten
koşulmuştur**, "derlendi" değil.

Bu koşum Faz 5'ten **bağımsız, önceden var olan bir kırmızı test** ortaya çıkardı:
`board_image_local_reads_file_metadata`, yerel bir imajın hiç init format sunmadığını iddia
ediyordu; oysa kod SD hedefi için iki BeagleBoard formatını sunuyor (ve `update_init_format`
tam da bunun için var). Değişikliklerim stash'lenip test tekrar koşularak aynı satırda aynı
şekilde başarısız olduğu doğrulandı — yani benim değişikliğim kaynaklı değil. Assertion koda
uygun hâle getirildi; T3 formatlarının yerel imajlarda **sunulmadığı** ayrıca belgelendi.

### Gerçek donanım

Bu fazda **hiçbir gerçek kart yazılmadı**. Aşağıdakiler donanım kabul matrisine aittir ve bu
raporda doğrulanmış sayılmaz:

- Üretilen `config.ini`'nin gerçek T3 boot'unda `gem-first-boot` tarafından uygulanması.
- `userpasswd` hash'iyle karta giriş yapılabilmesi.
- Türetilen PSK ile kartın ağa bağlanması.
- VNC değerinin kartın VNC sunucusunca kabul edilmesi.

---

## 3. Açık kalanlar

1. **DFU/staging yolu (§10.4).** "İmajı staging dosyasına açıp doğrulayıp özelleştirme", staging
   dosyasının başarı/hata/iptal/restart'ta temizlenmesi ve temp izinlerinin yalnız mevcut
   kullanıcıya açılması **yapılmadı** — DFU backend'i Faz 7/8'in konusu ve henüz yok. Serializer
   bu yolu bekleyecek şekilde `sd`'den bağımsız feature'a alındı.
2. **SDK: VNC sırrı temizlenmiyor.** `gem-first-boot` `$vncpassword` okuyor ama temizlikte
   `sed -i '/vncpasswd=/d'` yapıyor. İsimler uyuşmuyor, sır boot bölümünde kalıyor. §10.5 gereği
   ayrı SDK işi olarak raporlanır. İmager bunu gizlemiyor: kullanıcıya uyarı gösteriliyor. Ürün
   kararı VNC'yi tamamen kapatmak yönünde olursa tek satırlık değişiklik yeterli.
3. **SDK: `config.ini` hâlâ `source` ediliyor.** İmager tarafı tek başına savunulabilir
   (whitelist + katı doğrulama + reddetme) ve M1'i kapatır; ama consumer'ın dosyayı `source`
   etmek yerine parse etmesi asıl düzeltmedir.
4. **Desktop tespiti isim tabanlı.** `CustomizationProfile.desktop_variant` imaj adında "desktop"
   arıyor; katalogda açık bir bayrak yok. VNC alanlarını *sunmayı* kapılıyor, yazmayı değil.
   Katalog bir gün açık alan yayımlarsa oraya taşınmalı.
5. **Yerel imajlarda T3 özelleştirmesi yok.** Yerel bir `.img` T3 imajı olabilir ama bunu bilmenin
   yolu yok; `config.ini`'yi rastgele bir imaja yazmak tam olarak bu fazın önlemeye çalıştığı hata.
   Ürün kararı gerekiyorsa kullanıcıya açık bir "bu bir T3 imajı" seçimi eklenebilir.
6. **`gem-first-boot` kaynağı doğrudan okunmadı.** Desteklenen anahtar kümesi `planOptCl.md` §3.6
   tablosundan alındı. Tablo SDK kaynağından çıkarılmıştı; yine de kümeyi SDK'nın güncel HEAD'ine
   karşı bir kez daha doğrulamak ucuz ve değerli.

---

## 4. Commit serisi

| Commit | Kapsam |
|---|---|
| `45af0e9` | `feat(flasher)`: shell-safe serializer, sır tipleri, türevler, `VerifiedData` geri okuma, partition rewind düzeltmesi |
| `9e7a372` | `feat(config,gui)`: `InitFormat::GemInit`/`GemInitDesktop`, bridge eşlemesi, T3 paneli, persist kuralları |

İkisi de DCO sign-off taşıyor (`git commit -s`).

### Değişen dosyalar

**Yeni:** `bb-flasher/src/t3_gem_init/{mod,shell,secret,crypt}.rs`

**Değişen:** `bb-flasher/Cargo.toml` · `bb-flasher/src/lib.rs` ·
`bb-flasher/src/flasher/sd/mod.rs` · `bb-flasher-sd/src/{customization,lib}.rs` ·
`bb-flasher-sd/src/flashing/tests.rs` · `bb-config/src/config.rs` ·
`bb-config/src/t3/bridge.rs` · `bb-imager-gui/src/{persistance,helpers,main}.rs` ·
`bb-imager-gui/src/ui/configuration.rs` · `Cargo.lock`

### Yeni bağımlılıklar

Hepsi `t3_gem_init` feature'ına bağlı, hepsi olgun RustCrypto kuşağından (digest 0.10) seçildi ki
workspace tek bir `sha2`/`digest` sürümü paylaşsın:
`sha-crypt 0.5` · `pbkdf2 0.12` · `hmac 0.12` · `sha1 0.10` · `des 0.8` · `cipher 0.4` ·
`zeroize 1` · `getrandom 0.3` · `chrono-tz 0.10`.

`sha-crypt`'in `simple` feature'ı yalnız `[dev-dependencies]`'te: bağımsız doğrulayıcı
`sha512_check` oradan geliyor. Üretim kodu onu hiç çağırmıyor — hash, o crate'in RNG'siyle değil
OS CSPRNG salt'ıyla kuruluyor.

### Çalışma ağacındaki ilgisiz değişiklikler

`bb-helper/src/{lib,reader_progress}.rs` ve `bb-imager-gui/src/constants.rs` bu oturumdan önce de
değişikti (rustfmt kaynaklı satır düzenlemeleri). Çalışma kuralı 2 gereği **korundu**, commit
edilmedi. `bash.exe.stackdump` takip edilmeyen dosya olarak duruyor.
