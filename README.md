# Gateway Audio Lab

A native Rust desktop application that generates configurable stereo binaural tones and procedural noise inspired by the audio sequence described in the 9 June 1983 *Analysis and Assessment of Gateway Process* report (CIA document CIA-RDP96-00788R001700210016-5).

This is a research-oriented audio generator, not an official Monroe Institute product and not a recreation of the original Gateway tapes.

## What the report actually specifies

The report describes a staged listening exercise:

1. A preceding resonant-tuning exercise in which the participant hums along with a chorus on the tape.
2. An affirmation repeated by the participant.
3. Introduction of Hemi-Sync sound frequencies over stereo headphones.
4. Progressive physical relaxation while pink and white noise are added to the sound stream.

In its explanation of binaural beats, the report gives an illustrative example: a frequency in the left ear that is 10 Hz lower than an audible frequency in the right ear. The listener is said to perceive the difference as a beat frequency. The report does **not** provide a complete production recipe: it does not specify the original recording's exact carrier frequencies, a full beat-frequency progression, stage durations, gain levels, noise mix, or the underlying audio masters.

Primary source: [CIA-RDP96-00788R001700210016-5.pdf](https://www.cia.gov/readingroom/docs/CIA-RDP96-00788R001700210016-5.pdf). An accessible scan copy is available [here](https://upload.wikimedia.org/wikipedia/commons/d/dc/Analysis_and_Assessment_of_Gateway_Process.pdf). The binaural-beat example appears in the report's Frequency Following Response discussion (printed page 5); the sequence of tuning, affirmation, binaural tones, and pink/white noise appears in the Gateway Method discussion (printed page 19).

## Included features

- Live stereo playback through the default system audio device using CPAL.
- Separate left and right carrier frequencies. By default, left is 200 Hz and right is 210 Hz, giving the report's 10 Hz illustrative difference.
- A preceding equal-channel synthetic tuning tone. Its pitch and duration are application defaults, not values published in the report; the synthesizer does not recreate the human humming/chorus.
- Procedural white noise, pink noise, or a surf-like noise bed. The surf-like mode is generated noise with slow amplitude modulation, not a recording.
- The report describes a spoken/repeated affirmation, but the application does not synthesize speech or include a recording of it.
- Adjustable session duration, fade-in/fade-out, tone level, and noise level.
- Pause, resume, stop/reset, session progress, and stage display.
- Background WAV export as stereo 16-bit PCM. Rendering and the save dialog run outside the GUI/audio callback.
- Unit tests for the signal relationship, tuning stage, timing, and output bounds.

## Important interpretation notes

- The 10 Hz value is the report's numerical illustration, **not** an established prescription for a particular Focus state or a guarantee of brainwave entrainment.
- The report separately mentions 4–7 Hz mechanical/acoustical vibration and 6.8–7.5 Hz resonance in its discussion of Bentov's model. Those passages are not detailed binaural-audio settings, so the application does not mislabel them as such.
- This app is not scientifically validated as a way to produce Hemi-Sync, alter consciousness, enable remote viewing, or produce medical benefits. Those are historical claims discussed in the document, not established outcomes of this software.
- Binaural perception requires independent signals reaching the two ears; use stereo headphones. A mono audio path cannot preserve the intended channel separation.
- Start at a low listening volume. Do not use during driving or machinery operation. Stop if listening causes discomfort.

## Build and run

Requires Rust **1.95 or newer** (the current eframe 0.37 release MSRV) and a working desktop audio output device.

\`\`\`bash
cargo run --release
\`\`\`

Run unit tests and formatting checks:

\`\`\`bash
cargo test
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
\`\`\`

### Linux dependencies

CPAL uses the Linux ALSA development headers at build time. Install the equivalent packages for your distribution.

\`\`\`bash
# Arch Linux / CachyOS
sudo pacman -S --needed alsa-lib pkgconf

# Debian / Ubuntu
sudo apt install libasound2-dev pkg-config libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev

# Fedora
sudo dnf install alsa-lib-devel pkgconf-pkg-config
\`\`\`

The native save dialog uses the XDG Desktop Portal when available (and can fall back to \`zenity\` on Linux).

## Real-time and threading model

- CPAL owns the high-priority output callback thread.
- The GUI communicates live settings to that callback using atomic values; the callback does not acquire a mutex, allocate, or wait on a channel.
- Audio generation is performed directly into the device-provided output buffer.
- WAV rendering and the native save dialog run on a worker thread. The GUI polls completion with a non-blocking channel read.

## Project layout

\`\`\`text
Cargo.toml
README.md
rust-toolchain.toml
src/
  main.rs      # native window entry point
  app.rs       # eframe GUI and worker coordination
  audio.rs     # CPAL output stream and atomic controls
  dsp.rs       # stereo tone/noise synthesis and tests
  export.rs    # background WAV rendering function
\`\`\`

## License

MIT. The report itself is a U.S. government document; this application's source code is a separate work.
