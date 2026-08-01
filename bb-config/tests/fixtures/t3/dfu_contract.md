# T3 DFU behavior contract

The behavior is derived from the reference implementation in `gem-imager/src/dfuthread.cpp` and `dfuwrapper.cpp`/`.h`.

## Required stage order

| Order | Artifact | DFU alt-setting | Completion condition |
|---:|---|---|---|
| 1 | `tiboot3.bin` | `bootloader` | detach/reset and re-enumeration |
| 2 | `tispl.bin` | `tispl.bin` | detach/reset and re-enumeration |
| 3 | `u-boot.img` | `u-boot.img` | detach/reset and re-enumeration |
| 4 | extracted/customized `.img` | `rawemmc` | ZLP, terminal manifest state, and final detach |

## Transfer requirements

- Re-enumeration uses a deadline; the reference behavior allows up to 15 one-second attempts.
- The implementation waits for enumeration events between boot stages instead of relying on a fixed delay.
- `rawemmc` transfers use the transfer size reported by the device.
- Large images are streamed rather than loaded fully into memory.
- Raw eMMC flush and manifest completion may take up to 300 seconds.
- A zero-length DFU download packet follows the final data block.
- Success requires `dfuIDLE` or `dfuMANIFEST_WAIT_RST`.
- The final detach triggers U-Boot's `board_dfu_complete()` path.
- A stage is not complete until the next alt-setting enumerates.

## Mock transport event script

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

## Regression targets

The legacy `bb-flasher-dfu` implementation contains several behaviors that contract tests must cover:

1. The error-handling condition around `tiboot3.bin` does not match its comment.
2. A successful `tispl.bin` transfer can return early before `u-boot.img` and `rawemmc` are written.
3. Image sizes are narrowed from `u64` to `u32` with `unwrap()`.
4. USB context creation uses `unwrap()`.

Each defect requires a failing regression test before its implementation is changed.