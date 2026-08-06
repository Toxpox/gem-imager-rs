# libwdi runtime provenance

Gem Imager dynamically loads a WinUSB-only build of libwdi. No runtime download is performed.

| Item | Pinned value |
|---|---|
| Upstream release | `v1.5.1` |
| Upstream commit | `9b23b82a2dd1cbffc16d46c212f92c6bf8c0c602` |
| Source archive | `libwdi-v1.5.1.zip` |
| Source SHA-256 | `D74D27FDDBF5546C6A22A00FB67F9FC61A60B4AD9A7E974E9875E9CEE39BFAC7` |
| Pinned x64 DLL SHA-256 | `C9F0AAA5A1B0A71B1740256168E3F0A870E979149765F0E2778B160377B69F27` |
| Microsoft WDF redistributable SHA-256 | `29314207814CE9D5D73695F7E9239539CF37C79E750B9D5EA5A5EF5487A583D6` |

Upstream source: <https://github.com/pbatard/libwdi/releases/tag/v1.5.1>

Microsoft redistributable used for the embedded WinUSB/WDF 1.11 co-installers:
<https://download.microsoft.com/download/0/5/F/05FD6919-6250-425B-86ED-9B095E54065A/wdfcoinstaller.msi>

The build disables libusb-win32, libusbK and ARM64 payloads. It retains x86 and x64 installer
helpers because the x64 libwdi runtime requires the matching embedded installer and supports an
x86 payload when preparing a package. The only upstream-source changes are applied by
`scripts/build-libwdi.ps1`: build-path configuration, disabling those unrelated payloads, and
removing project references after the prerequisite helper projects are built explicitly.

`COPYING-LGPL` is the upstream LGPL notice. `Microsoft-WDF-License.rtf` is the redistributable
license extracted from Microsoft's package. Keep this directory, the source archive, notices and
the dynamically linked DLL together in source and release compliance reviews.

The committed DLL is the reviewed artifact. A rebuild may have a different hash because compiler,
linker, resource and timestamp inputs can differ; update the Rust allowlist only after reviewing a
new build and its exported symbols.
