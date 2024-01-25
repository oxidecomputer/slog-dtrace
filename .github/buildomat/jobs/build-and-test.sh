#!/bin/bash
#:
#: name = "helios / build-and-test"
#: variety = "basic"
#: target = "helios"
#: rust_toolchain = "1.75"
#: output_rules = []
#:

set -o errexit
set -o pipefail
set -o xtrace

cargo --version
rustc --version

#
# We build with:
#
# - RUSTFLAGS="-D warnings" RUSTDOCFLAGS="-D warnings": disallow warnings
#   in CI builds.  This can result in breakage when the toolchain is
#   updated, but that should only happen with a change to the repo, which
#   gives us an opportunity to find and fix any newly-introduced warnings.

# - Work-around for cargo#9895 via RUSTDOCFLAGS also.
#
export RUSTFLAGS="-D warnings"
export RUSTDOCFLAGS="-D warnings"
banner fmt
ptime -m cargo fmt -- --check
banner test
ptime -m cargo test --verbose -- --test-threads 1
