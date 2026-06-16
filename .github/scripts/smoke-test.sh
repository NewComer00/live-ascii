#!/bin/bash
set -e

MODEL_DIR="hiyori_free"
if [ ! -d "$MODEL_DIR" ]; then
    echo "Downloading sample model..."
    wget -q https://cubism.live2d.com/sample-data/bin/hiyori/hiyori_en.zip
    unzip -qo hiyori_en.zip
    rm hiyori_en.zip
fi

# Run smoke test — the binary enters a TUI loop after loading the model.
# timeout kills it and returns 124
TIMEOUT_SEC=5
set +e
timeout --foreground $TIMEOUT_SEC socat -T $TIMEOUT_SEC - EXEC:"cargo run --release -- '$MODEL_DIR/runtime/hiyori_free_t08.model3.json'",pty,setsid,ctty
EXIT_CODE=$?
set -e

if [ $EXIT_CODE -eq 124 ]; then
    echo "PASS: model loaded and running (exit $EXIT_CODE)"
    exit 0
else
    echo "FAIL: binary exited with code $EXIT_CODE"
    exit 1
fi
