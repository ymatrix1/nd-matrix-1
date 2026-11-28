# The plugin's sound library

The CLAP/VST3 plugin plays the programs of a Nord Stage 4 backup (`.ns4b`): the factory restore file from Nord, or your own backup with the pianos and sample banks it uses. The editor's OPEN .NS4B button does that by hand; this page is the hands-free way, for a DAW that cannot show the plugin window (SoundCraft on Windows) and for projects that must sound the same when reopened.

## 1. Save the library once

```bash
ns4 library set "F:\path\to\Nord Stage 4 Factory Restore RevK v1.66.ns4b"
ns4 library status
```

`set` opens the file and says how many programs it holds; `clear` forgets it. Only the path is stored, in `ns4/config.json` under your user config folder next to the Nord OS update setting (`ns4 firmware set`). The environment variable `NS4_LIBRARY` overrides the saved path.

Give the plugin the Nord OS update too (`ns4 firmware set`, see [Where to get the Nord OS update](nord-os-update.md)): WAVE sounds then use the real wavetables.

## 2. Pick a program in the DAW

Every instance has a host parameter **Program**: `Off`, then `A:11` .. `H:88` (the locations the keyboard shows). A new instance starts at `A:11`, the first program of the library. Change it from the DAW's parameter list or an automation lane; the plugin decodes that program with its piano and sample banks on a worker thread and switches to it at the next audio block. Large pianos take a few seconds to decode.

The value is saved with the project, so a reopened project loads the same program again. Selecting `Off` releases the library program: the plugin drops it and its sound banks, and the host parameters drive the engine again, as in a plugin without a library. A project saved with `Off` reopens with `Off` (no `A:11` is loaded when the host restores the saved value before it activates the plugin; when it restores it after, the program that loaded in between is released at the next block). The panel keeps the last program's values for the settings no host parameter covers until the instance is reset or reloaded.

**Projects saved before this parameter existed** store no value for it, so they open at the default `A:11` and load the library's first program over their own sound the first time the library is saved. To keep such a project as it was, set `Program` to `Off` and save it again, or do not save a library.

## What to expect

- Adding the instrument, or opening a project, waits a few seconds while the program's banks decode (a factory grand piano is 218 MB); instances of the same program share one decode. The wait stops after 15 seconds and the program then arrives while the host runs. `NS4_LIBRARY=none` in the host's environment switches the library off for that run.
- A library file replaced while the host runs (a new `ns4 library set` target at the same path, or a re-export) is read again at the next program request; the banks decoded from the old file are let go.
- The editor opens the saved library's directory when its window is created, on the host's UI thread: 32 ms for a synthetic zip of 512 programs and 5 000 sound entries (debug build, `editor/library_open_tests.rs`). Sounds are not read until a program asks for them.

- No library saved, or the file moved: the plugin starts and plays its built-in sounds; `ns4 library status` says why.
- What the plugin loaded, and why a load failed, is appended to `plugin.log` next to the settings file (`%APPDATA%\ns4\plugin.log` on Windows); the file is emptied once it passes 1 MB.
- The DAW must be running the plugin (playback or monitoring on) for a load to apply; a load waits up to 30 seconds for that.
- If the Nord OS update tables are not in after 5 seconds, the program plays its WAVE sounds from the placeholder tables; a later program change picks the real ones up, and `plugin.log` says so.
- A program whose piano or sample bank is not in the library plays without that layer, like the keyboard's "Piano not found".
- Moving a knob of a group (organ, synth, ...) after a load gives that group back to the host parameters, as in the editor (D218).
- In a host that does show the plugin window, the Library & Backup tab of the editor opens on the saved library, so its program list and the `Program` parameter name the same sounds.
- The desktop app (`nd-matrix-1`) opens the same saved library at start when it is given no `--backup`, and loads its first program `A:11` when no `--program` is given, so the display shows a program and the keys sound wherever it is launched from. What it found is appended to `app.log` next to the settings file.
