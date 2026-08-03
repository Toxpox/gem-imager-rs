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

Install a current stable Rust toolchain and Git LFS, then clone and run the application:

```sh
git lfs install
git clone https://github.com/Toxpox/gem-imager-rs.git
cd gem-imager-rs
cargo run -p bb-imager-gui
```

The command-line interface is also available:

```sh
cargo run -p bb-imager-cli -- --help
```

## Development

The workspace uses feature-gated components. Use the Makefile targets to check and test the same combinations used by CI:

```sh
make check
make test
```

For T3 Gemstone software, images, and documentation, visit [t3gemstone.org](https://t3gemstone.org/en) and the [official documentation](https://docs.t3gemstone.org).

## License

This project is available under the [MIT License](LICENSE).
