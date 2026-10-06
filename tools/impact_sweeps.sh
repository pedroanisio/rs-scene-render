#!/usr/bin/env bash
# The sweeps of the two impact scenes, as one table: what each scene gives for the speeds, masses and angles of
# the monotonicity tests, with the ocean answering to the bed and the bodies by the depth of the water
# (`Filtered`, the default) and by the hydrostatic pressure (`Hydrostatic`). Run it again when the ocean changes;
# every line that begins with IMPACT is a measurement, and the tests that assert an order fail if it does not hold.
#
# usage: tools/impact_sweeps.sh [test-name-filter]   (a test of the scene file, or the path of one under it: the
# filter follows `impact_scenes::`, which is that file in the `crater` test binary of sr-eval)
set -euo pipefail
filter="${1:-}"
# SR_CARGO_PROFILE picks the build profile (release by default; ci for test runs without LTO).
CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" cargo test --profile "${SR_CARGO_PROFILE:-release}" -p sr-eval --test crater -- \
    --nocapture --include-ignored --skip cost_of "impact_scenes::$filter" 2>&1 |
    grep -E '^IMPACT|^test .*(FAILED|failed)|^test result|panicked'
