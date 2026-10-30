# verse

Hold a hotkey, talk, let go: a tiny pill appears at the bottom of the screen while you speak, and
what you said is typed into whatever input has focus. Everything runs locally with
[whisper.cpp](https://github.com/ggerganov/whisper.cpp).

Nothing runs between uses. Each key press starts the process and the release ends it, so idle
memory is zero. While recording, the overlay itself is a raw Wayland layer surface drawn in
software (no toolkit, no GPU context); the memory in use then is almost all the model.

While you talk, a bubble above the pill shows what whisper hears so far. It re-transcribes the
recording about every half second; when you let go, one final pass over everything is what gets
typed. Turn it off under "Live transcript" in the menu if your machine is slow (each pass
costs a full transcription, which is cheap on a GPU and heavy on a CPU).

Settings live in a small menu that opens right above the pill when you click its button. It is
drawn the same way, by egui laying it out and a CPU rasterizer painting it, so the whole process,
pill and menu included, stays around 20 MB.

## Requirements

- A wlroots-style compositor with `wlr-layer-shell` (Hyprland, Sway, river, …)
- [`wtype`](https://github.com/atx/wtype) to type the text
- `notify-send` (optional) for error messages
- `wl-clipboard` (optional) for copy and paste in the menu

## Install

On Arch, build and install the package from this checkout:

```sh
cd packaging
makepkg -si
```

That installs `/usr/bin/verse` (built with Vulkan) and a "Verse" launcher entry that opens the
settings. `pacman -R verse` removes it again.

## Build

```sh
cargo build --release                    # CPU
cargo build --release --features vulkan  # GPU via Vulkan (NVIDIA and AMD)
cargo build --release --features cuda    # GPU via CUDA
```

## Use

```
verse start     # show the overlay and record (bind to key press)
verse stop      # transcribe and type it (bind to key release)
verse toggle    # start, or stop if already recording
verse cancel    # discard the recording
verse settings  # open the menu without recording
verse settings vocabulary   # … or at a page: models, language, vocabulary, replacements
```

Click the button on the left of the pill to open the menu. It closes with Escape, its ×, another
click on the button, or by clicking somewhere else after you've used it. The first `start`
without a downloaded model opens it at the model list.

### Hyprland

Hold `SUPER + SHIFT` to talk (Lua config):

```lua
hl.bind("SUPER + Shift_L", hl.dsp.exec_cmd("verse start"))
-- Released, Shift is no longer held, so the release ignores modifiers to match at all.
hl.bind("Shift_L", hl.dsp.exec_cmd("verse stop"), { release = true, ignore_mods = true })
-- The same with SHIFT pressed first.
hl.bind("SHIFT + Super_L", hl.dsp.exec_cmd("verse start"))
hl.bind("Super_L", hl.dsp.exec_cmd("verse stop"), { release = true, ignore_mods = true })
```

Classic `hyprland.conf`:

```
bind   = SUPER, Shift_L, exec, verse start
bindri = , Shift_L, exec, verse stop
```

`SUPER + SHIFT` is also the prefix of other shortcuts. Using one of them makes a short tap,
and taps under 0.3 s are dropped without transcribing, so they only flash the pill.

To blur the backgrounds: `layerrule = blur, verse` (the pill), `verse-menu` and `verse-transcript` (the live bubble).

## Models

Downloaded from [ggerganov/whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp) into
`~/.local/share/verse/models/`.

| Model | Size | When |
| --- | --- | --- |
| `large-v3-turbo-q5_0` | 547 MB | Default. Near large-v3 accuracy, multilingual. Fast with a GPU |
| `small` / `small.en` | 466 MB | Good accuracy on a CPU |
| `base` / `base.en` | 142 MB | Fast on any CPU |
| `tiny.en` | 75 MB | Weak hardware |

`.en` models are English only and a little more accurate for English.

## Development

`VERSE_FAKE_MIC=sample.wav verse start` replays a 16 kHz mono 16-bit WAV in real time instead of
recording the microphone, for trying changes without talking.

## Unusual words and code

Whisper turns words it doesn't know into common English ("kubectl" becomes "cube cuddle").
Verse fixes this in three steps:

1. **Vocabulary as prompt.** Your vocabulary is passed to whisper as its initial prompt, which
   makes those spellings much more likely. Whisper reads only about 224 tokens of prompt, so
   keep the list to a few dozen terms.
2. **Matching by sound.** Words whisper still gets wrong are compared to the vocabulary by
   sound (Double Metaphone), so "cube cuddle" becomes `kubectl`. It only ever outputs
   vocabulary terms. A single heard word must also be spelled almost the same, so "roast" stays
   "roast" when `Rust` is in the list.
3. **Replacements.** Fixed rewrites for what's left, like `pseudo` → `sudo`.

All three run without a model and add no noticeable time.

The menu offers vocabulary presets (Linux & shell, Web development, Rust, Python, DevOps &
cloud, AI & ML). They list what whisper misspells, like identifiers, tools and brands, and not
plain words it already gets right. Your own words come first in the prompt, then the presets.

## Config

`~/.config/verse/config.toml`, written by the menu:

```toml
model = "large-v3-turbo-q5_0"
language = "auto"   # or "en", "de", …
theme = "dark"      # or "light"
max_seconds = 120   # recording stops on its own after this
use_gpu = true
live_preview = true
vocabulary = ["kubectl", "useEffect", "Hyprland"]
presets = ["linux", "rust"]   # linux, web, rust, python, devops, ai

[[replacements]]
from = "pseudo"
to = "sudo"
```
