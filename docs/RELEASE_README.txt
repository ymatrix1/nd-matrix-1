ND Matrix 1 - release archive
=======================================

Source-available (non-commercial) software emulator, DSP engine and
binary-format toolkit for the Nord Stage 4 keyboard. Built for
interoperability, research and preservation.
Source: https://github.com/ymatrix1/nd-matrix-1

Licensed under PolyForm Noncommercial 1.0.0: commercial use is not permitted.
See the LICENSE file in this archive.

Status: early release (AS IS, no warranty or support)
----------------------------------------------------

This software is provided "AS IS", without warranty or support of any kind.
There is no technical support, SLA, or guarantee of fitness for any purpose.

The CLAP and VST3 plugin has NOT been verified in any DAW yet. Automated tests
cover the plugin code and the engine, but nobody has loaded this build in
Reaper, Bitwig, Ableton Live or another host and checked audio, MIDI routing,
automation and state recall. Expect rough edges. See HOST_COMPATIBILITY.md in
this archive for the current matrix.

The builds are unsigned. The archive contains no firmware, factory sounds or
sample data from the hardware; sounds come from the built-in engine and from
files you load yourself.

Contents
--------

  ns4                       command-line tool (inspect presets, offline render)
  nd-matrix-1               standalone GUI (audio + MIDI)
  nd-matrix-1-standalone    standalone host of the plugin build
  nd-matrix-1.clap          CLAP plugin
  nd-matrix-1.vst3          VST3 plugin (a folder bundle; copy the whole folder)
  HOST_COMPATIBILITY.md     what has and has not been tested
  README.txt                this file

(Windows binaries end in .exe.)

Verify the download
-------------------

Every archive has a .sha256 file on the release page (SHA256SUMS.txt holds all of
them). Put the archive and its .sha256 file in one folder and run:

  Linux:    sha256sum -c <archive>.sha256
  macOS:    shasum -a 256 -c <archive>.sha256
  Windows:  (Get-FileHash <archive> -Algorithm SHA256).Hash
            and compare it with the hash in the .sha256 file.

A checksum detects a damaged download; it is not a signature.

Install the plugin
------------------

Copy the plugin into one of these folders, then rescan plugins in your DAW.

Windows (x86_64)
  CLAP, all users:   C:\Program Files\Common Files\CLAP\nd-matrix-1.clap
  CLAP, this user:   %LOCALAPPDATA%\Programs\Common\CLAP\nd-matrix-1.clap
  VST3, all users:   C:\Program Files\Common Files\VST3\nd-matrix-1.vst3\
  VST3, this user:   %LOCALAPPDATA%\Programs\Common\VST3\nd-matrix-1.vst3\
  The VST3 folder must keep its layout:
  nd-matrix-1.vst3\Contents\x86_64-win\nd-matrix-1.vst3

macOS (Apple silicon, arm64)
  CLAP, all users:   /Library/Audio/Plug-Ins/CLAP/nd-matrix-1.clap
  CLAP, this user:   ~/Library/Audio/Plug-Ins/CLAP/nd-matrix-1.clap
  VST3, all users:   /Library/Audio/Plug-Ins/VST3/nd-matrix-1.vst3
  VST3, this user:   ~/Library/Audio/Plug-Ins/VST3/nd-matrix-1.vst3
  The builds are not signed or notarized. If macOS refuses to open them, clear
  the quarantine flag on the files you installed, for example:
    xattr -dr com.apple.quarantine ~/Library/Audio/Plug-Ins/VST3/nd-matrix-1.vst3
    xattr -dr com.apple.quarantine ~/Library/Audio/Plug-Ins/CLAP/nd-matrix-1.clap
  Do the same for ns4, nd-matrix-1 and nd-matrix-1-standalone before running them.

Linux (x86_64)
  CLAP, this user:   ~/.clap/nd-matrix-1.clap
  CLAP, all users:   /usr/lib/clap/nd-matrix-1.clap
  VST3, this user:   ~/.vst3/nd-matrix-1.vst3/
  VST3, all users:   /usr/lib/vst3/nd-matrix-1.vst3/
  The VST3 folder must keep its layout:
  nd-matrix-1.vst3/Contents/x86_64-linux/nd-matrix-1.so
  The GUI needs ALSA and X11/OpenGL libraries; the plugin needs a host that
  provides its own window.

Upgrading from an older release: earlier builds installed the plugin as
nord-plugin.clap and nord-plugin.vst3. The installers and the AppImage
--install-plugins option remove them. If you copy files by hand, delete the old
nord-plugin.clap and nord-plugin.vst3 so your DAW does not list two copies.
Saved projects keep loading: the plugin identity did not change.

Run the standalone programs
---------------------------

  ns4 info
  ns4 render-demo --frames 48000
  nd-matrix-1

The programs are not installed anywhere; run them from the extracted folder.
