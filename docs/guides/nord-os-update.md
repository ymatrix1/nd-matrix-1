# Where to get the Nord OS update

Some Nord Stage 4 sounds (the WAVE oscillators) use wavetables that live inside the keyboard's operating system. They are Nord's data, so this project does not ship them. You download the official update from Nord yourself, give that one file to the app, and the app reads the tables from it every time it starts. Nothing is copied out of the file and nothing is uploaded.

Without the file everything still works; WAVE sounds use a built-in placeholder wavetable.

## 1. Download

Visit the official Nord Stage 4 Downloads page (<https://www.nordkeyboards.com/products/nord-stage-4/downloads>, also reachable from <https://www.nordkeyboards.com/> → Downloads → Nord Stage 4 → OS Update) and download the official "Update Nord Stage 4 OS" file for your platform:

| Your computer | File |
|---|---|
| Windows | `Update Nord Stage 4 OS v1.66.zip` (or the `.exe` inside it) |
| Mac | `Update Nord Stage 4 OS v1.66.dmg` |

Use either file as downloaded. Do not unzip it, open the `.dmg`, or run the updater. Only OS v1.66 is supported today; other versions say "unsupported Nord OS version".

## 2. Give it to the app

**App (`nd-matrix-1`):** click "Nord OS update file...", drop the downloaded file on the window (or paste its path) and press "Use this file". The window shows how many WAVE tables were read. Restart the app once.

**Command line (`ns4`):**

```bash
ns4 firmware set "C:\Users\you\Downloads\Update Nord Stage 4 OS v1.66.zip"
ns4 firmware status
```

`set` checks the file first and says "this is not a Nord OS update" when it is the wrong file. `ns4 firmware clear` forgets it.

## What is remembered

Only the path of the file, in `ns4/config.json` under your user config folder (Windows `%APPDATA%`, macOS `~/Library/Application Support`, Linux `~/.config`). Keep the downloaded file where it is; if you move it, set it again. If the saved file is moved, deleted or damaged, the app and `ns4` still start: they print or show "saved Nord OS update file not used" and use the placeholder wavetable until you set it again. A broken `config.json` is reported the same way. `nd-matrix-1`, `ns4` and the plugin use the saved file (the plugin reads it when an instance activates and switches its WAVE stage on once a library program is loaded); `nord-tools` does not.

Order of precedence for power users: `--synth-wave-tables PATH`, then the environment variable `NS4_WAVE_TABLES`, then the saved path, then the placeholder. `--no-synth-wave-bank` turns the firmware tables off for one run.
