# T3 DFU davranış sözleşmesi (Faz 0 sabiti)

Kaynak: `instruction.md` §3.2 ve §12.3. Bu dosya sözleşmeyi Faz 7 testleri için sabit referans olarak taşır; `gem-imager` kaynağı bu sözleşmenin değişmemesi gereken davranışsal oracle'ıdır (`gem-imager/src/dfuthread.cpp`, `dfuwrapper.cpp`/`.h`).

## Aşama sırası (kesin, atlanamaz)

| Sıra | Artifact | DFU alt-setting | Aşama sonu |
|---:|---|---|---|
| 1 | `tiboot3.bin` | `bootloader` | detach/reset ve yeniden enumerate |
| 2 | `tispl.bin` | `tispl.bin` | detach/reset ve yeniden enumerate |
| 3 | `u-boot.img` | `u-boot.img` | detach/reset ve yeniden enumerate |
| 4 | açılmış/özelleştirilmiş `.img` | `rawemmc` | ZLP, manifest terminal durumu, son detach |

## Parity ayrıntıları

- Deadline tabanlı yeniden arama; referans davranışta en fazla 15 × 1 saniye deneme.
- Boot aşamaları arası referans bekleme 2 saniye; yeni kod sabit sleep yerine enumerate olayını beklemeli.
- `rawemmc` aktarımında cihazın bildirdiği transfer boyutu kullanılır.
- Büyük imaj tamamen belleğe alınmaz (streaming).
- Raw eMMC flush/manifest için 300 saniyeye kadar bekleme.
- Son veri bloğundan sonra sıfır uzunluklu (ZLP) DFU download paketi.
- Başarıdan önce `dfuIDLE` veya `dfuMANIFEST_WAIT_RST` görülmesi zorunlu.
- Son detach ile U-Boot `board_dfu_complete()` yolu tetiklenir.
- Yeni alt-setting enumerate olmadan önceki aşama başarılı kabul edilmez.

## Mock transport event script örneği (§12.3)

```text
Enumerated(bootloader)
DownloadOk
Disconnected
Enumerated(tispl.bin)
DownloadOk
Disconnected
Enumerated(u-boot.img)
DownloadOk
Disconnected
Enumerated(rawemmc)
ChunksAccepted
State(dfuMANIFEST)
State(dfuMANIFEST_WAIT_RST)
DetachOk
```

## Bilinen mevcut hata (regresyon test hedefi, düzeltme değil)

`bb-flasher-dfu/src/lib.rs:132-140` (baseline HEAD `ab3570059264324ce38783e51021855b3693ddeb`'e göre gözlemlendi):

```rust
// For some reason tiboot3 does not exit properly. So need to ignore errors.
if img.0.as_str() != "tiboot3.bin" {
    match res {
        Err(Error::DownloadFail { .. }) => {}
        _ => return res,
    }
} else {
    res?;
}
```

Gözlenen kusurlar:

1. Yorum davranışın tersini söylüyor: hata yutma `tiboot3.bin` için değil, diğer tüm artifact'ler için uygulanıyor.
2. `_ => return res` `tispl.bin` başarılı olduğunda (`Ok(())`) fonksiyonun erken dönmesine yol açıyor; `u-boot.img` ve `rawemmc` hiç yazılmıyor olabilir.
3. `lib.rs:118` ve `lib.rs:128`'de `size.try_into().unwrap()` (u64→u32) — instruction.md §19 yasağı.
4. `flashing.rs:17`'de `rusb::Context::new().unwrap()`.

Faz 7 §12.1 gereği: değişiklikten önce bu hatayı gösteren regresyon testi yazılmalı, sonra typed state machine ile yeşile döndürülmeli.
