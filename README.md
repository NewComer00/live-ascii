# Live-ASCII

A Live2D Cubism model renderer for the terminal, with optional face tracking.

![showcase](./showcase.gif)

## Supported Platforms

| Platform | x86_64 | arm64 |
|----------|:------:|:-----:|
| Windows* |   ✅    |       |
| macOS    |        |   ✅   |
| Linux    |   ✅    |   ✅   |

\* **Windows:** build on [MSYS2 UCRT64](https://www.msys2.org/). The release binary runs both on Windows and MSYS2 UCRT64.

## Usage

Live-ASCII uses [PurismCore](https://github.com/SakuraMotion/PurismCore), an MIT-licensed reimplementation of Live2D Cubism Core (API v6). PurismCore is automatically fetched and built on first `cargo build` — no manual setup required.

**Prerequisites:** `git` and `make` (and a C compiler like `cc`) must be available on your PATH. On Windows, use the MSYS2 UCRT64 shell to build.

### Quick Start

Download the [Niziiro Mao](https://www.live2d.com/en/learn/sample/niziiro-mao/) English sample ([mao_en.zip](https://cubism.live2d.com/sample-data/bin/mao/mao_en.zip)), unzip into `./models/mao_en/`, then run (see [Live2D sample terms](https://www.live2d.com/en/learn/sample/model-terms/)):

```bash
mkdir -p models/mao_en
curl -L -o mao_en.zip https://cubism.live2d.com/sample-data/bin/mao/mao_en.zip
unzip -q mao_en.zip -d models/mao_en
cargo run --release -- models/mao_en/runtime/mao_pro.model3.json
```

More sample models are listed [here](https://www.live2d.com/en/learn/sample/).

### Display & Rendering

#### Unicode Halfblock

```bash
cargo run --release -- models/mao_en/runtime/mao_pro.model3.json
```

#### Sixel

```bash
cargo run --release -- models/mao_en/runtime/mao_pro.model3.json --image-protocol sixel
```

Lower sixel encode resolution:

```bash
cargo run --release -- models/mao_en/runtime/mao_pro.model3.json --image-protocol sixel --sixel-resolution 40%
```

```bash
cargo run --release -- models/mao_en/runtime/mao_pro.model3.json --image-protocol sixel --sixel-resolution 4x8
```

`--sixel-resolution` (sixel only) sets quantette resolution. Output is upsampled to the reference display size (10×20 px per terminal cell):

| Value | Quantette px/cell |
|-------|-------------------|
| `100%` (default) | 10×20 |
| `50%` | 5×10 |
| `40%` | 4×8 |
| `10x20` | 10×20 |
| `4x8` | 4×8 |

Percent values without `%` are also accepted (e.g. `40` = `40%`).

Color quality presets:

```bash
cargo run --release -- models/mao_en/runtime/mao_pro.model3.json --image-protocol sixel --sixel-color-quality high
```

```bash
cargo run --release -- models/mao_en/runtime/mao_pro.model3.json --image-protocol sixel --sixel-color-quality epic
```

`--sixel-color-quality` (sixel only) — game-style presets (minimum 64 colors). Diffusion is Floyd–Steinberg error strength (0.0–1.0):

| Preset | Colors | Diffusion | Quantizer | Notes |
|--------|--------|-----------|-----------|-------|
| `low` or `fast` | 64 | 0.5 | Wu | smallest palette |
| `medium` or `mid` | 128 | 0.5 | Wu | |
| `high` (default) | 256 | 0.375 | Wu | balanced; lighter dither for live animation |
| `ultra` | 256 | 0.875 | Wu | |
| `max` or `best` | 256 | 1.0 | K-means | slowest, strongest dither |

#### Kitty

```bash
cargo run --release -- models/mao_en/runtime/mao_pro.model3.json --image-protocol kitty
```

#### Background / View Scale / Offset

```bash
cargo run --release -- models/mao_en/runtime/mao_pro.model3.json --bg-color "rgba(30,30,30,255)" --scale "200%" --offsetx "-10%" --offsety "50%"
```

| Flag | Values | Description |
|------|--------|-------------|
| `--bg-color` | `rgba(r,g,b,a)` | Background behind the character; not applied in sixel mode |
| `--scale` | e.g. `"200%"` | Initial view scale (default `100%`) |
| `--offsetx`, `--offsety` | e.g. `"-10%"`, `"50%"` | Initial view offset as % of panel size |

### Model Features

Face tracking (requires OpenSeeFace — see below):

```bash
cargo run --release -- models/mao_en/runtime/mao_pro.model3.json --camera
```

Physics simulation:

```bash
cargo run --release -- models/mao_en/runtime/mao_pro.model3.json --physics
```

Mouse — drag to pan, scroll to zoom:

```bash
cargo run --release -- models/mao_en/runtime/mao_pro.model3.json --mouse
```

Combine flags:

```bash
cargo run --release -- models/mao_en/runtime/mao_pro.model3.json --camera --physics --mouse --image-protocol halfblock --bg-color "rgba(0,0,0,0)" --scale "300%" --offsety "50%"
```

### Face Tracking

Ensure [OpenSeeFace](https://github.com/emilianavt/OpenSeeFace) is running and sending data to the default UDP port (`11573`).

### VTube Studio API

Live-ASCII can act as a [VTube Studio Public API](https://github.com/DenchiSoft/VTubeStudio) server so standard VTS clients (pyvts, vtubestudio-rs, VTubeStudioJS, etc.) can drive the model without real VTS running.

**Enable the server:**

```bash
cargo run --release -- models/mao_en/runtime/mao_pro.model3.json --vts
```

Custom port:

```bash
cargo run --release -- models/mao_en/runtime/mao_pro.model3.json --vts --vts-port 8002
```

Require manual plugin approval (default: auto-approve):

```bash
cargo run --release -- models/mao_en/runtime/mao_pro.model3.json --vts --no-vts-auto-approve
```

| Flag | Default | Description |
|------|---------|-------------|
| `--vts` | off | Start the VTS-compatible WebSocket server |
| `--vts-port` | `8001` | WebSocket listen port (same default as VTube Studio) |
| `--vts-auto-approve` | `true` | Auto-grant plugin authentication tokens (logged to stderr) |

The server binds to `127.0.0.1` only. Approved plugin tokens are stored in `~/.live-ascii/vts_tokens.json`. Do not run real VTube Studio on the same port at the same time.

**Client flow:** connect → `APIStateRequest` → `AuthenticationTokenRequest` → `AuthenticationRequest` → API calls (e.g. `InjectParameterDataRequest` at ≥1 Hz for owned parameters).

VTS tracking names such as `FaceAngleX` map to Live2D parameters (e.g. `ParamAngleX`). Clients may also inject Live2D parameter IDs directly. When both `--vts` and `--camera` are enabled, VTS-injected parameters take priority over OpenSeeFace for the same Live2D param.

| Category | Messages |
|----------|----------|
| Session | `APIStateRequest`, `AuthenticationTokenRequest`, `AuthenticationRequest`, `StatisticsRequest` |
| Parameters | `InputParameterListRequest`, `Live2DParameterListRequest`, `ParameterValueRequest`, `InjectParameterDataRequest` |
| Model | `CurrentModelRequest`, `AvailableModelsRequest` |
| Actions | `HotkeysInCurrentModelRequest`, `HotkeyTriggerRequest`, `ExpressionStateRequest`, `ExpressionActivationRequest` |

Hotkeys are built from expressions and motions in the loaded model (and optional `live.json` entries).

**Test client (vtubestudio-rs):**

Terminal 1:

```bash
cargo run --release -- models/mao_en/runtime/mao_pro.model3.json --vts
```

Terminal 2:

```bash
cargo run --example vts_inject_loop
```

The example authenticates as `"live-ascii test"` / `"Dev"` (auto-approved by default), then drives head/face tracking, triggers motion hotkeys, and toggles expressions. Open the debug Parameters panel (`D` with a `live.json` hotkey) to verify injected values.

| Variable | Default | Purpose |
|----------|---------|---------|
| `VTS_URL` | `ws://127.0.0.1:8001` | WebSocket URL if using a custom port |
| `VTS_AUTH_TOKEN` | (none) | Reuse a saved token; a new one is printed on first auth |

**Example (Python / pyvts):**

Connect, authenticate, then inject tracking parameters in a loop. `FaceAngleX` is a VTS input name. Live-ascii maps it to the Live2D param (e.g. `ParamAngleX`). Stop with Ctrl+C.

Requires `pip install pyvts`. Start live-ascii with `--vts` first.

```python
import asyncio
import math
import time

import pyvts


async def main():
    # Create a VTS plugin instance with metadata shown in the VTube Studio UI.
    # authentication_token_path: where the auth token is cached between runs.
    vts = pyvts.vts(
        plugin_info={
            "plugin_name": "My Plugin",
            "developer": "Developer",
            "authentication_token_path": "./pyvts_token.txt",
        }
    )

    # Open the WebSocket connection to VTube Studio (default: ws://localhost:8001).
    await vts.connect()

    # Request a fresh auth token from VTube Studio.
    # force=True ignores any cached token in pyvts_token.txt — necessary when
    # switching between a real VTS instance and a mock server, since the old
    # token won't be recognized by the new host.
    await vts.request_authenticate_token(force=True)

    # Send the token back to VTube Studio to complete the handshake.
    # After this call the session is authorized and API requests are accepted.
    await vts.request_authenticate()

    t0 = time.monotonic()
    try:
        while True:
            # Compute a sinusoidal head-tilt angle in degrees, oscillating
            # between -30° and +30° with a period of ~6.3 seconds (2π seconds).
            angle = 30.0 * math.sin(time.monotonic() - t0)

            # Inject the value into the Live2D parameter FaceAngleX.
            # face_found=True tells VTS to treat this as a valid tracking frame;
            # without it VTS may fall back to its default idle behavior.
            await vts.request(
                vts.vts_request.requestSetParameterValue(
                    "FaceAngleX", angle, face_found=True
                )
            )

            # Throttle to ~30 Hz. VTube Studio's internal parameter injection
            # loop also runs at 30 Hz, so sending faster would be wasteful;
            # sending slower would cause visible stuttering.
            await asyncio.sleep(1 / 30)
    finally:
        # Always close the WebSocket cleanly so VTS deregisters the plugin.
        await vts.close()


asyncio.run(main())
```

### Controls

**Keyboard:**

| Key | Action |
|-----|--------|
| `↑` `↓` `←` `→` | Move the character |
| `+` / `=` | Zoom in |
| `-` | Zoom out |
| `q` | Quit |

**Mouse** (pass `--mouse` to enable):

| Gesture | Action |
|---------|--------|
| Drag | Pan the view |
| Scroll up | Zoom in |
| Scroll down | Zoom out |

## Interactions

To use in-app hotkeys (motion panel, debug panel, physics toggle, camera), add a `model_name.live.json` next to your `model_name.model3.json`:

```json
{
  "Version": 1,
  "Name": "your_model_name",
  "Hotkeys": [
    {
      "Action": "Open/Close Motion Panel",
      "Triggers": {
        "Trigger1": "M",
        "Trigger2": "",
        "Trigger3": ""
      }
    },
    {
      "Action": "Open/Close Debug Panel",
      "Triggers": {
        "Trigger1": "D",
        "Trigger2": "",
        "Trigger3": ""
      }
    },
    {
      "Action": "Enable/Disable Physics",
      "Triggers": {
        "Trigger1": "P",
        "Trigger2": "",
        "Trigger3": ""
      }
    },
    {
      "Action": "Open/Close Camera",
      "Triggers": {
        "Trigger1": "C",
        "Trigger2": "",
        "Trigger3": ""
      }
    }
  ]
}
```