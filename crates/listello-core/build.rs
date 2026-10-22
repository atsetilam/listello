// Retained from tiny-dfr (https://github.com/AsahiLinux/tiny-dfr)
// Copyright The Asahi Linux Contributors
// SPDX-License-Identifier: MIT
//
// Unmodified. Moved here from build.rs at the workspace root.

fn main() {
    pkg_config::probe_library("fontconfig").unwrap();
}
