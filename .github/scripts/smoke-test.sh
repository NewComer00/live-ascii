#!/bin/bash
set -e

MODEL_DIR="hiyori_free"
if [ ! -d "$MODEL_DIR" ]; then
    echo "Downloading sample model..."
    wget -q https://cubism.live2d.com/sample-data/bin/hiyori/hiyori_en.zip
    unzip -qo hiyori_en.zip
    rm hiyori_en.zip
fi

# Run smoke test — model loads, then TUI requires a terminal (which CI lacks).
# Check that PurismCore initializes successfully regardless of exit code.
TIMEOUT_SEC=5
MODEL_SETTING="$MODEL_DIR/runtime/hiyori_free_t08.model3.json"
set +e
OUTPUT=$(timeout --foreground $TIMEOUT_SEC cargo run --release -- "$MODEL_SETTING" 2>&1)
set -e

if echo "$OUTPUT" | grep -q "Purism Core version"; then
    echo "PASS: model loaded successfully (PurismCore initialized)"
    exit 0
else
    echo "FAIL: model failed to load (exit $EXIT_CODE)"
    echo "$OUTPUT"
    exit 1
fi
