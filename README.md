# Gateway Audio Lab

A native Rust desktop application that generates configurable stereo binaural tones and procedural noise inspired by the audio sequence described in the 9 June 1983 *Analysis and Assessment of Gateway Process* report (CIA document CIA-RDP96-00788R001700210016-5).

This is a research-oriented audio generator, not an official Monroe Institute product and not a recreation of the original Gateway tapes. The native desktop window uses eframe/egui with its winit backend.

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
- Optional **Harmonic Visualization** native window, opened/closed with a checkbox in the main application.
- Seven selectable visualization modes: live dashboard, stereo phase, waveforms, FFT spectrum, cymatic mandala, phase interference field, and harmonic geometry rings.
- The visualizer samples the signal actually sent to CPAL through a fixed-size atomic ring buffer; the audio callback does not lock, allocate, or wait for the visualizer.
- Closing the visualization window with its title-bar close button synchronizes the checkbox in the main window.
- Unit tests for the signal relationship, tuning stage, timing, output bounds, sample-tap snapshots, and FFT carrier-frequency detection.

## Important interpretation notes

- The 10 Hz value is the report's numerical illustration, **not** an established prescription for a particular Focus state or a guarantee of brainwave entrainment.
- The report separately mentions 4–7 Hz mechanical/acoustical vibration and 6.8–7.5 Hz resonance in its discussion of Bentov's model. Those passages are not detailed binaural-audio settings, so the application does not mislabel them as such.
- This app is not scientifically validated as a way to produce Hemi-Sync, alter consciousness, enable remote viewing, or produce medical benefits. Those are historical claims discussed in the document, not established outcomes of this software.
- Binaural perception requires independent signals reaching the two ears; use stereo headphones. A mono audio path cannot preserve the intended channel separation.
- Start at a low listening volume. Do not use during driving or machinery operation. Stop if listening causes discomfort.

## Build and run

Requires Rust **1.95 or newer** (the eframe 0.37 release MSRV) and a working desktop audio output device.

```bash
cargo run --release
```

Run unit tests and formatting checks:

```bash
cargo test
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

### Linux dependencies

CPAL uses the Linux ALSA development headers at build time. Install the equivalent packages for your distribution.

```bash
# Arch Linux / CachyOS
sudo pacman -S --needed alsa-lib pkgconf

# Debian / Ubuntu
sudo apt install libasound2-dev pkg-config libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev

# Fedora
sudo dnf install alsa-lib-devel pkgconf-pkg-config
```

The native save dialog uses the XDG Desktop Portal when available (and can fall back to `zenity` on Linux).

## Harmonic visualizer

Enable **Open harmonic visualization in a separate window** in the main window. The visualization window can be moved, resized, or closed independently; closing it also turns the main-window toggle off.

The window now includes a **Visualization** selector with seven modes:

- **Live dashboard:** stereo phase plot, left/right waveforms, and measured FFT spectrum together.
- **Stereo phase / Lissajous:** plots captured left-channel samples against right-channel samples.
- **Stereo waveforms:** independently inspect recent left and right output samples.
- **Frequency spectrum:** Hann-windowed 16,384-point FFT for each channel, displayed in dBFS from 0 to 1 kHz.
- **Cymatic mandala:** animated radial geometry whose lobe count and modulation respond to the binaural difference, carrier, noise mix, and elapsed phase.
- **Phase interference field:** a two-source wave-interference model; the spatial pattern changes with the two channel frequencies, their difference, noise mix, and time.
- **Harmonic geometry rings:** a 16-order radial geometric model driven by the carrier ratio and phase.

The selected mode is remembered while the visualizer window remains open. Switch modes at any time without stopping audio. The measured views use the actual samples sent to the output device. The three geometry modes are mathematical visualization models, not measured acoustic cymatics, physical resonance experiments, or claims that a sine wave contains extra harmonics.

The spectrum needs 16,384 output frames before it can be populated (about 0.34 seconds at 48 kHz). The phase and waveform views appear as soon as samples are available.

**Important signal distinction:** the current synthesizer generates sine-wave carriers plus optional noise. A sine wave has a fundamental frequency but does not itself contain a rich harmonic series. The spectrum therefore shows the carrier peaks and any broadband noise that is actually present; it does not invent overtones. The 10 Hz binaural difference is a separation between the two ear channels, not an extra acoustic 10 Hz tone in either channel.

## Real-time and threading model

- CPAL owns the high-priority output callback thread.
- The GUI communicates live settings to that callback using atomic values; the callback does not acquire a mutex, allocate, or wait on a channel.
- Audio generation is performed directly into the device-provided output buffer.
- WAV rendering and the native save dialog run on a worker thread. The GUI polls completion with a non-blocking channel read.

## Project layout

```text
Cargo.toml
README.md
rust-toolchain.toml
src/
  main.rs          # native window entry point
  app.rs           # eframe GUI, export worker, and visualizer viewport
  audio.rs         # CPAL output stream, atomic controls, and sample tap
  dsp.rs           # stereo tone/noise synthesis and tests
  visualization.rs # lock-free tap, FFT analysis, and visualizer drawing
  export.rs        # background WAV rendering function
```

## License

MIT. The report itself is a U.S. government document; this application's source code is a separate work.
