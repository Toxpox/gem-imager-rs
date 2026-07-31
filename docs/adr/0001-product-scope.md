# ADR 0001: Ürün kapsamı — T3-GEM-O1 ve BeagleY-AI

- Durum: Kabul edildi (güvenli varsayılan)
- Tarih: 2026-07-31
- Referans: `instruction.md` §5.2, §3.1

## Bağlam

`instruction.md` iki kapsam seçeneği tanımlar:

- `T3Only`: UI ve paket yalnız T3-GEM-O1 gösterir.
- `T3AndBeagleY`: T3-GEM-O1 SD+DFU, BeagleY-AI SD-only gösterilir.

Bu proje `bb-imager-rs` forkundan türetilmiştir; kaynak ağaç BeagleY-AI dahil çok sayıda BeagleBoard kartını destekler (`config.json` içindeki `devices` listesi: Generic Linux Board, BeagleConnect Freedom, BeagleY-AI, BeaglePlay, BeagleBone AI-64, ...). T3 Gemstone Imager'ın ürün kararı bu oturumda kullanıcı tarafından açıkça verilmemiştir.

## Karar

**`T3Only`** seçildi — `instruction.md` §5.2'nin öngördüğü güvenli varsayılan.

- UI ve paket metadata'sı yalnız T3-GEM-O1'i gösterir.
- Katalog adaptörü BeagleY (veya başka) girdisi gördüğünde panic etmez; açıkça "ürün kapsamı dışında" sınıflandırır ve gözlemlenebilir tanı (JSON path + sebep) üretir.
- Bu karar açık bir ürün onayıyla `T3AndBeagleY`'e yükseltilene kadar geçerlidir.

## Sonuç — Faz 2 budaması üzerindeki bağlayıcılık

`instruction.md` §7 son maddesi gereği: bu ADR `T3Only` dese bile, **BeagleY-AI asset ve kodu geri döndürülemez biçimde silinemez.** `T3Only` yalnız görünürlüğü daraltır (UI, paket, dokümantasyon); kaynak ağacını daraltmaz. Açık ürün kararıyla `T3AndBeagleY`'e geçiş ihtimali kaynak seviyesinde korunmalıdır — örn. BeagleY board tanımı/asset'leri kod tabanından silinmez, yalnız varsayılan ürün yüzeyinden çıkarılır.

Kapsam dışı olarak Faz 2'de tamamen budanabilecek parçalar (BeagleY'den bağımsız, karara duyarlı olmayan): `bb-flasher-bcf`, `bb-flasher-mspm0`, `bb-flasher-pb2-mspm0`, T3 yayın formatında kullanılmayan `bb-bmap-parser`/bmap writer yolu, `SdCardBootfs`/`UpdateBootFlasher`, ilgisiz CLI alt komutları.

## Yeniden gözden geçirme tetikleyicileri

- T3 Gemstone ürün sahibinden BeagleY-AI desteği için açık onay gelmesi.
- Katalog adaptörünün gerçek T3 kataloğunda hiç BeagleY girdisi görmediğinin doğrulanması (bu durumda `T3AndBeagleY` kod yolu yalnız teorik kalır, kaldırılması ayrıca değerlendirilebilir).
