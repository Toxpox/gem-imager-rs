# ADR 0001: Ürün kapsamı — T3-GEM-O1 ve BeagleY-AI

- Durum: **Kabul edildi — `T3AndBeagleY`** (ürün sahibi onayı, 2026-08-02)
- Tarih: 2026-07-31 (ilk kayıt), 2026-08-02 (karar güncellendi)
- Referans: `instruction.md` §5.2, §3.1

## Bağlam

`instruction.md` iki kapsam seçeneği tanımlar:

- `T3Only`: UI ve paket yalnız T3-GEM-O1 gösterir.
- `T3AndBeagleY`: T3-GEM-O1 SD+DFU, BeagleY-AI SD-only gösterilir.

Bu proje `bb-imager-rs` forkundan türetilmiştir; kaynak ağaç BeagleY-AI dahil çok sayıda BeagleBoard kartını destekler (`config.json` içindeki `devices` listesi: Generic Linux Board, BeagleConnect Freedom, BeagleY-AI, BeaglePlay, BeagleBone AI-64, ...).

İlk kayıtta ürün kararı verilmemişti; bu yüzden `instruction.md` §5.2'nin öngördüğü güvenli varsayılan olan `T3Only` geçici olarak seçilmişti.

## Karar

**`T3AndBeagleY`** — ürün sahibi 2026-08-02'de T3-GEM-O1 ve BeagleY-AI'ın birlikte destekleneceğini açıkça onayladı. Bu, ilk kayıttaki geçici `T3Only` varsayılanının yerini alır.

- UI ve paket metadata'sı **T3-GEM-O1 ve BeagleY-AI**'ı gösterir.
- **Yazma yeteneği kart başına ayrışır ve kataloğun tek bir flasher alanından okunmaz:**
  - T3-GEM-O1 → SD **ve** DFU üzerinden onboard eMMC,
  - BeagleY-AI → **yalnız SD**. Doğrulanmış bir DFU profili olmadığı için DFU hedefi hiçbir koşulda
    önerilmez (`bb-config/src/t3/canonical.rs`, `BoardCapabilities`; GUI tarafında
    `helpers::WriteMethods::resolve`).
- Katalog adaptörü kapsam dışı bir girdi gördüğünde (örn. tagsız "No filtering" sözde-cihazı) panic
  etmez; açıkça "ürün kapsamı dışında" sınıflandırır ve gözlemlenebilir tanı (JSON path + sebep)
  üretir.

### Uygulama noktası

Kapsam, kataloğun doğrulandığı tek yerde uygulanır:

```
bb-imager-gui/src/helpers.rs — fetch_remote_config()
    bb_config::t3::ProductScope::T3AndBeagleY
```

`ProductScope::T3Only` varyantı ve onu kapsayan testler kaldırılmaz: kapsamın daraltılması ileride
tekrar gündeme gelebilir ve daralttığında ne olacağının test edilmiş olması gerekir.

## Sonuç — Faz 2 budaması üzerindeki bağlayıcılık

`instruction.md` §7 son maddesi gereği **BeagleY-AI asset ve kodu geri döndürülemez biçimde silinemez.** Bu kısıt, kapsam artık `T3AndBeagleY` olduğu için evleviyetle geçerlidir: BeagleY-AI artık yalnız korunan değil, **desteklenen** bir karttır.

Kapsam dışı olarak Faz 2'de tamamen budanabilecek parçalar (BeagleY'den bağımsız, karara duyarlı olmayan): `bb-flasher-bcf`, `bb-flasher-mspm0`, `bb-flasher-pb2-mspm0`, T3 yayın formatında kullanılmayan `bb-bmap-parser`/bmap writer yolu, `SdCardBootfs`/`UpdateBootFlasher`, ilgisiz CLI alt komutları.

## Bu kararın açık bıraktıkları

- **Kabul testi kapsamı genişler:** BeagleY-AI artık desteklenen bir kart olduğu için SD yazma,
  geri okuma ve gerçek boot kabulü T3'ün yanı sıra BeagleY-AI için de kanıtlanmalıdır
  (minimal ve desktop imaj aileleri). Canlı katalogda 17 `beagley-ai` imajı vardır.
- **Ürün metinleri güncellenmelidir:** `bb-imager-gui/Cargo.toml` açıklaması hâlen yalnız
  "your T3 Gemstone board" diyor; paket metadata'sı ve README bu kararı yansıtmalıdır.

## Yeniden gözden geçirme tetikleyicileri

- Ürün sahibinin BeagleY-AI desteğini geri çekmesi (`T3Only`'ye dönüş).
- BeagleY-AI için doğrulanmış bir DFU profilinin ortaya çıkması — bu durumda kartın SD-only kısıtı
  yeniden değerlendirilir.
