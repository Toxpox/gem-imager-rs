# gem-helper

Common helper utilities used across the BeagleBoard imaging tools.

This crate is a small shared library that provides:

- A file-backed stream splitter (`file_stream`) that exposes an async writer and a synchronous reader.
- A `Resolvable` trait (`resolvable`) for representing image sources that can be resolved to local files.

## Features

- `file_stream` – enables `gem_helper::file_stream` and related types.
- `resolvable` – enables `gem_helper::resolvable` and related types.

## Usage

Add `gem-helper` as a dependency and enable the feature(s) you need:

```toml
[dependencies]
gem-helper = { path = "../gem-helper", features = ["file_stream", "resolvable"] }
```

Then use the available types in your crate:

```rust
#[cfg(feature = "file_stream")]
use gem_helper::file_stream;

#[cfg(feature = "resolvable")]
use gem_helper::resolvable::LocalFile;
```
