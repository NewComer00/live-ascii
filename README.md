# Live-ASCII

A Live2D Cubism model renderer for terminal. It also support face tracking.

![showcase](./showcase.gif)

## Supported Platforms

| Platform | x86_64 | arm64 |
|----------|:------:|:-----:|
| Windows* |   ✅    |       |
| macOS    |        |   ✅   |
| Linux    |   ✅    |   ✅   |

\* Windows support is tested on MSYS2 UCRT64.

## Usage

Live-ASCII uses [PurismCore](https://github.com/SakuraMotion/PurismCore), an MIT-licensed reimplementation of Live2D Cubism Core (API v6). PurismCore is automatically fetched and built on first `cargo build` — no manual setup required.

**Prerequisites:** `git` and `make` (and a C compiler like `cc`) must be available on your PATH.

```bash
cargo run --release -- ./path/to/model.model3.json
```

You can download and try Live2D sample model [here](https://www.live2d.com/en/learn/sample/).

```bash
# Run with camera tracking enabled
cargo run --release -- ./path/to/model.model3.json --camera

# Run with physics enabled
cargo run --release -- ./path/to/model.model3.json --physics

# Run with mouse support (drag to pan, scroll to zoom)
cargo run --release -- ./path/to/model.model3.json --mouse

# Run with Sixel graphics protocol
cargo run --release -- ./path/to/model.model3.json --image-protocol sixel

# Lower sixel quantette resolution for faster encode (same on-screen size)
cargo run --release -- ./path/to/model.model3.json --image-protocol sixel --sixel-resolution 40%
cargo run --release -- ./path/to/model.model3.json --image-protocol sixel --sixel-resolution 4x8

# Set background color behind the character (rgba format)
cargo run --release -- ./path/to/model.model3.json --bg-color "rgba(30,30,30,255)"

# Set view scale (default 100%)
cargo run --release -- ./path/to/model.model3.json --scale "200%"

# Set view offset as percentage of panel width/height (default 0%)
cargo run --release -- ./path/to/model.model3.json --offsetx "-10%" --offsety "50%"

# Combine flags
cargo run --release -- ./path/to/model.model3.json --camera --physics --mouse --image-protocol halfblock --bg-color "rgba(0,0,0,0)" --scale "300%" --offsety "50%"
```

`--image-protocol` values:

| Value | Description |
|-------|-------------|
| `halfblock` (default) | Unicode half-block characters — works in any terminal |
| `sixel` | Sixel graphics — needs a Sixel-capable terminal (xterm -ti 340, foot, WezTerm) |
| `kitty` | Kitty graphics protocol — Kitty, Konsole ≥ 23.04, WezTerm, Ghostty |

`--bg-color` accepts an `rgba(r,g,b,a)` string, e.g. `--bg-color "rgba(255,0,0,128)"`. Not applied in sixel mode (sixel always renders opaque to avoid frame bleed).

`--scale`, `--offsetx`, `--offsety` accept a percentage string like `"200%"`, `"-10%"`, `"50%"` and set the initial view transform.

`--sixel-resolution` (sixel mode only) sets quantette resolution. Output is always upsampled to the reference display size (10×20 px per terminal cell). Accepts a percentage or explicit px/cell:

| Value | Quantette px/cell | On-screen |
|-------|-------------------|-----------|
| `100%` (default) | 10×20 | reference size |
| `50%` | 5×10 | reference size |
| `40%` | 4×8 | reference size |
| `10x20` | 10×20 | reference size |
| `4x8` | 4×8 | reference size |

Percent values without `%` are also accepted (e.g. `40` = `40%`).

Note: *For face tracking, ensure [OpenSeeFace](https://github.com/emilianavt/OpenSeeFace) is running and sending data to the default UDP port (11573).*

## Basic Operations

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

In order to interact with app. You need to write a `model_name.live.json` file and place it in the same folder as your `model_name.model3.json`. <br>
examples: 
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

## Features in future
- Separate live2d framework to a crate.
- Support processes interaction.
- Enable multiply expressions.
