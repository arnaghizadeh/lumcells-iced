#!/usr/bin/env bash
# Builds the examples that the Lumcells patches add, runs their tests, and starts each one under
# Xvfb with the software renderer for a few seconds: it has to be alive at the end and must not
# have panicked. Used by .github/workflows/ci.yml and by hand (`.github/scripts/fork-examples.sh`).
set -euo pipefail
cd "$(dirname "$0")/../.."

# One line per example that a patch adds.
EXAMPLES=()
if [ -f .github/scripts/fork-examples.list ]; then
    mapfile -t EXAMPLES < <(grep -v '^\s*\(#\|$\)' .github/scripts/fork-examples.list)
fi
if [ ${#EXAMPLES[@]} -eq 0 ]; then
    echo "no examples yet"
    exit 0
fi

packages=()
for e in "${EXAMPLES[@]}"; do packages+=(--package "$e"); done
cargo build --locked "${packages[@]}"
# Under Xvfb: some of the tests start the built example
ICED_TEST_BACKEND=tiny-skia xvfb-run -a cargo test --locked "${packages[@]}"

target="${CARGO_TARGET_DIR:-target}/debug"
for e in "${EXAMPLES[@]}"; do
    log="$(mktemp)"
    # A bus and a display of its own, so that nothing shows on (or asks) the desktop of whoever runs this.
    set +e
    ICED_BACKEND=tiny-skia timeout --preserve-status -s TERM 20 \
        dbus-run-session -- xvfb-run -a -s "-screen 0 1280x800x24" \
        bash -c '"$0" & pid=$!; sleep 6; if kill -0 $pid 2>/dev/null; then kill $pid; wait $pid 2>/dev/null; exit 0; else wait $pid; echo "exited early with $?"; exit 1; fi' \
        "$target/$e" >"$log" 2>&1
    status=$?
    set -e
    if [ $status -ne 0 ] || grep -q "panicked at" "$log"; then
        echo "FAILED: $e"
        cat "$log"
        exit 1
    fi
    echo "ok: $e (alive after 6 s, no panic)"
done
