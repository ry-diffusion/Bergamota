#!/bin/sh
# Compiles Core/PerfCatalog.swift (with Core/Shell.swift for shellQuote) and Tests/perf-tests.swift, and runs them.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
cp "$here/perf-tests.swift" "$tmp/main.swift"    # top-level code is allowed only in main.swift
swiftc -O -o "$tmp/perf-tests" "$here/../Bergamota/Core/PerfCatalog.swift" "$here/../Bergamota/Core/Shell.swift" "$tmp/main.swift"
exec "$tmp/perf-tests"
