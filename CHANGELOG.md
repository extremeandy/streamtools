# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project (will try to) adhere to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

### Legend

The following icons are used to distinguish breaking changes from non-breaking changes.

- 🔥: Breaking change (high impact: will require code changes for most users)
- 💔: Breaking change (low impact: won't require code changes for most users)

## 0.7.6

### Added

- Added `merge_join_by` combinator

### Changed

- Minimum rust version bumped to `1.85.0` as part of moving to rust 2024 edition

### Security

- Optional dependency `tokio` bumped from a version with a [RustSec advisory](https://rustsec.org/advisories/RUSTSEC-2025-0023)


## 0.7.5

### Fixed

- Fix bug in `sample` where the sampler was not being polled until `Pending` when the inner value was not ready yet, which meant that the waker was not being called on the sampler

## 0.7.4

### Fixed

- Removed faulty constraints on `flat_map_switch` combinator

## 0.7.3

### Added

- Added `flat_map_switch` combinator

## 0.7.2

### Fixed

- Fixed an issue with `FlattenSwitch` where the inner stream could be polled again after termination.

## 0.7.0

### Added

- 💔 Added `ThrottleLast` combinator.
- Added "test-util" feature with some helper methods for testing such as `record_delay` and `delay_items`.

## 0.6.0

### Changed

- 🔥 Changed `Sample` combinator `Item` to be `T` instead of `Option<T>`. The stream will no longer yield `None` when the sampler yields and no value is available on the underlying stream.
