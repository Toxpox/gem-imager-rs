<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/t3gemstone/gem-imager/main/.meta/logo-dark.png" />
    <img src="https://raw.githubusercontent.com/t3gemstone/gem-imager/main/.meta/logo-light.png" alt="T3 Gemstone" width="360" />
  </picture>
</p>

# T3 Gemstone Imager

T3 Gemstone Imager is a Rust desktop application for downloading, configuring, and writing supported operating-system images to T3 Gemstone hardware.

It writes images to SD cards and supports verified DFU flashing to the onboard eMMC on T3-GEM-O1. Image downloads and writes are checked for integrity before they are used.

## Getting started

Install a current stable Rust toolchain with [rustup](https://rustup.rs), Git, Git LFS, and the
native build tools for your platform. Clone the repository and fetch its LFS-managed assets:

```sh
git lfs install
git clone https://github.com/Toxpox/gem-imager-rs.git
cd gem-imager-rs
git lfs pull
```

For a development run with the product feature set:

```sh
cargo run -p gem-imager-gui --features sd,dfu
```

The command-line interface is available with:

```sh
cargo run -p gem-imager-cli --features dfu -- --help
```

## Build and package

The Makefile builds release binaries with `--locked`. Its normal GUI feature set is `sd,dfu`; the
Windows x64 target additionally enables the reviewed WinUSB provisioning helper. Generated
installers and portable packages are written below `gem-imager-gui/dist/`.

### Debian and Ubuntu

These commands apply to supported Debian and Ubuntu releases on an x86_64 host. For an ARM64 host,
replace the target triple with `aarch64-unknown-linux-gnu`.

Install the common tools and repository dependencies:

```sh
sudo apt update
sudo apt install -y build-essential pkg-config make git git-lfs curl wget
git lfs install
make setup-debian-deps
rustup target add x86_64-unknown-linux-gnu
```

Build the GUI and CLI:

```sh
make build TARGET=x86_64-unknown-linux-gnu
```

The binaries are created in `target/x86_64-unknown-linux-gnu/release/`. To prepare a Debian/Ubuntu
installer package:

```sh
make setup-packaging-deps TARGET=x86_64-unknown-linux-gnu
make package-gui-deb TARGET=x86_64-unknown-linux-gnu
sudo apt install ./gem-imager-gui/dist/*.deb
```

To produce all supported x86_64 Linux artifacts, including the GUI `.deb`, AppImage, generic
archives, and CLI packages:

```sh
make package-x86_64-unknown-linux-gnu
```

An AppImage can also be built on its own with:

```sh
make package-gui-appimage TARGET=x86_64-unknown-linux-gnu
```

### macOS

DMG packages must be built on macOS. Install the Xcode command-line tools, Git LFS, and both Rust
targets when producing Intel and Apple Silicon artifacts:

```sh
xcode-select --install
brew install git-lfs
git lfs install
rustup target add x86_64-apple-darwin aarch64-apple-darwin
```

Build for the current Mac and create its DMG:

```sh
make build-gui
make setup-packaging-deps
make package-host
```

To build both architectures explicitly:

```sh
make package-x86_64-apple-darwin
make package-aarch64-apple-darwin
```

Open the generated `.dmg` from `gem-imager-gui/dist/` and drag **T3 Gemstone Imager** into
`Applications`. Local builds are unsigned unless Apple Developer signing and notarization
credentials are supplied; unsigned packages can trigger Gatekeeper warnings.

libusb is compiled in statically on macOS, so the bundle does not depend on a Homebrew install
at runtime. Packaging enforces this: it fails if the app links anything outside `/System`,
`/usr/lib`, or an `@rpath`-relative location, because such a library is missing on a clean Mac
and the app then fails to launch with only a generic "cannot be opened" dialog.

To audit a `.dmg` from any host, including Linux:

```bash
scripts/verify-macos-dmg.sh path/to/T3.Gemstone.Imager_*.dmg
```

### Windows

Use a 64-bit Windows host with the stable Rust MSVC toolchain, Git LFS, and Visual Studio 2022 Build
Tools with the **Desktop development with C++** workload. The commands below run in PowerShell and
do not require GNU Make.

```powershell
git lfs install
git lfs pull
rustup toolchain install stable --profile minimal
rustup target add x86_64-pc-windows-msvc
```

Build the consoleless WinUSB helper and GUI using the same feature set as the x64 release package:

```powershell
cargo build -r --locked -p gem-winusb-helper --target x86_64-pc-windows-msvc
cargo build -r --locked -p gem-imager-gui --target x86_64-pc-windows-msvc `
  --features sd,dfu,dfu-driver-mvp,updater,pre-release,notify-rust
```

The executable is created at
`target/x86_64-pc-windows-msvc/release/gem-imager-gui.exe`. Install the pinned packager and create
a WiX MSI:

```powershell
cargo install cargo-packager --locked --version 0.11.8
cargo packager --config Packager.windows-x64.toml -r --verbose -f wix
```

To prepare the portable ZIP with the GUI, consoleless helper, reviewed `libwdi.dll`, and license
files:

```powershell
$metadata = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
$version = ($metadata.packages | Where-Object { $_.name -eq 'gem-imager-gui' }).version
.\scripts\package-windows-portable.ps1 -Version $version -Target x86_64-pc-windows-msvc
```

Both the MSI and portable ZIP are written to `gem-imager-gui/dist/`. Locally generated Windows
packages are not Authenticode-signed and can trigger Microsoft Defender SmartScreen.

If GNU Make is installed, the equivalent complete x64 package target is:

```powershell
make setup-packaging-deps package-x86_64-pc-windows-msvc
```

## Validation

The workspace uses feature-gated components. On macOS and Linux, use the Makefile targets to check
both the standalone/default GUI configuration and the feature combinations used by CI:

```sh
make check
make test
```

On Windows without GNU Make, the core equivalents are:

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo test -p gem-imager-gui
cargo test -p gem-imager-gui --features sd,dfu
```

For T3 Gemstone software, images, and documentation, visit [t3gemstone.org](https://t3gemstone.org/en) and the [official documentation](https://docs.t3gemstone.org).

## License

This project is available under the [MIT License](LICENSE).
