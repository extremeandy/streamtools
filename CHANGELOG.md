# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project (will try to) adhere to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

### Legend

The following icons are used to distinguish breaking changes from non-breaking changes. 

- 🔥: Breaking change (high impact: will require code changes for most users)
- 💔: Breaking change (low impact: won't require code changes for most users)

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
