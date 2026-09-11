# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0](https://github.com/water-rs/waterui/compare/waterui-browser-cef-v0.1.0...waterui-browser-cef-v0.2.0) - 2026-09-11

### Fixed

- *(hydrolysis)* make direct_to_target work on HiDPI, one format, opaque only

### Other

- *(deps)* vello_cpu 0.2 and the remaining tooling bumps
- *(cef)* take Chromium's message loop out of the render callback
- *(core)* give the default-accessibility-role rule one home
- *(webview)* decouple the Chromium CDP session from waterui-webview
