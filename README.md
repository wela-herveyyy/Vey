<p align="center"><img src="assets/logo.png" width="320" alt="Vey"></p>

<h1 align="center">Vey</h1>

<p align="center">A tiny, native <b>notch</b> and <b>top bar</b> made for Windows.<br>
Written in Rust. No webview, no framework, no runtime. One small <code>.exe</code> (~1 MB), or a tiny installer.</p>

<p align="center">
<a href="https://github.com/wela-herveyyy/Vey/actions/workflows/ci.yml"><img src="https://github.com/wela-herveyyy/Vey/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
<a href="https://github.com/wela-herveyyy/Vey/releases/latest"><img src="https://img.shields.io/github/v/release/wela-herveyyy/Vey" alt="Latest release"></a>
<img src="https://img.shields.io/badge/platform-Windows%2010%2F11-blue" alt="Windows 10/11">
<img src="https://img.shields.io/badge/license-MIT-green" alt="MIT">
</p>

<p align="center"><img src="docs/demo.gif" width="760" alt="Vey demo: the notch opens, tabs are shown, top bar dropdowns, buddy and scenes"></p>

<p align="center"><a href="https://github.com/wela-herveyyy/Vey/releases/latest"><b>⬇ Download for Windows 10/11</b></a> &nbsp;·&nbsp; free, open source, MIT &nbsp;·&nbsp; about 1 MB</p>

---

> [!NOTE]
> ### Honoring the Original: Built upon Nock
> **Vey** is an enhanced continuation and evolution of [**Nock**](https://github.com/AkhileshVarute/Nock), created by [**Akhilesh Varute**](https://github.com/AkhileshVarute). 
> 
> We extend our deepest gratitude to Akhilesh for designing, architecting, and open-sourcing the incredible zero-idle-cost native Rust notch foundation that powers this project. Vey builds forward on that foundation with expanded productivity workflows, modernized UI design systems, and external suite integrations.

---

## What's New in Vey

1. **Controls Tile Design System**:
   - Modern borderless matte cards (`SURF` / `HOVER`) resting on the black notch canvas.
   - Tinted circular icon badges with semantic accent colors and crisp Segoe Fluent / MDL2 icons.
   - Standardized two-line stacked typography and strict 2-column grid alignment across tabs.

2. **Craft Suite Integration (Apps Tab)**:
   - Native discovery, installation, and launching for the **ArtCraft** creative tools:
     - **Craft Launcher**: Central craft hub and environment launcher.
     - **GridCraft**: Ultra-fast, pure Rust spreadsheets.
     - **DeckCraft**: Native presentations rebuilt in Rust.
     - **WordCraft**: Lightweight, fast word processor in pure Rust.
   - Real-time download progress, installation states, and direct executable detection.

3. **Sprint Backlog & Work Tracker (Work Tab)**:
   - Built-in agile sprint backlog integrated with ERPNext.
   - Priority indicators, task status toggles, assignee drop-downs, and instant browser deep links.

4. **Pixel Art Brand Identity**:
   - Brand new retro-tech wordmark logo, launcher icon, and system tray iconography crafted in pixel art.

---

## Features

- **Notch** at the top centre. Click it and a panel opens with tabs: **Media**, **Controls**, **Today** (focus timer, to-dos, calendar), **Work** (agile backlog), **Stats**, **Clipboard**, **Shelf** (drop files on it), **Apps** (pinned apps & Craft Suite), **Games**, and **Power**.
- **Top bar** along the top of the screen: active app name, Wi-Fi, Bluetooth, sound, brightness, battery, and clock, each with a rich dropdown (join Wi-Fi networks, connect Bluetooth devices, volume/brightness sliders, calendar). Optional clock styles and CPU / memory read-outs.
- **A little life**: a desktop buddy character on the bar, scripted scenes, an arcade with four mouse-only games, and small tactile reactions on the pill (squish on click, glow with music, confetti when the focus timer ends).
- **Smart pill**: the notch proactively displays context without a click:
  - Amber "Battery low" with time remaining.
  - "Mic on" indicator with duration and one-click global microphone mute.
  - "Link / Path / Colour / Code copied" (click a link or path to open Explorer/browser).
  - Break nudges after 90 minutes of continuous keyboard/mouse activity.
  - Live meeting countdowns from your calendar (`.ics`).
- **Idle notch extras** (all optional): glance text next to the clock (tasks left, focus minutes today, CPU/RAM, or next meeting), focus progress lines, workday progress lines, and scroll-up cycling.
- **Zero idle cost**: event-driven, zero polling loops; ~0% CPU and ~32 MB memory while idle. Hides automatically during full-screen games and video with top-edge cursor reveal.
- Follows Windows light/dark theme dynamically.

---

## Download & Install

### Windows (10 & 11)
Download **`VeySetup-<version>.exe`** from the [latest release](../../releases/latest) and run it.
- Installs into your user profile (no administrator privileges needed).
- Adds Vey to the Windows Start menu so you can search for "Vey".
- Can start automatically with Windows (Tray icon → **Start with Windows**).

Prefer a portable binary? Download **`Vey-portable-<version>.exe`** from the same release: place it anywhere and run.

> [!NOTE]
> **Windows SmartScreen Note**: Vey is open source and not yet code-signed with an expensive commercial certificate. If Windows shows *"Windows protected your PC"*, click **More info → Run anyway**.

### Linux (Preview)
Vey runs on Linux using the same native Rust core.
- **Debian / Ubuntu**: Download `vey_<version>_amd64.deb` from releases and run:
  ```bash
  sudo apt install ./vey_<version>_amd64.deb
  ```
- **Tarball**: Download `vey-<version>-x86_64-linux.tar.gz`, unpack, and run `./install.sh`.
- Requires an X11 session or XWayland, plus `fonts-dejavu-core`.

---

## Building from Source

### On Windows
1. Install Rust via [rustup.rs](https://rustup.rs) (default MSVC toolchain).
2. Install Visual Studio Build Tools with the **"Desktop development with C++"** workload (provides the Windows SDK to embed resources and icons).
3. Clone and build:
   ```cmd
   git clone https://github.com/wela-herveyyy/Vey.git
   cd Vey
   cargo build --release
   ```
4. Output binary is at `target\release\vey.exe` — a single self-contained ~1 MB executable.

*(Optional)* To compile the installer wizard yourself, install [Inno Setup 6](https://jrsoftware.org/isinfo.php) and run:
```cmd
ISCC.exe /DMyAppVersion=1.2.3 installer\vey.iss
```

### On Linux
```bash
sudo apt install build-essential fonts-dejavu-core
cargo build --release
packaging/linux/package.sh
```

---

## Using Vey

| Action | What happens |
| --- | --- |
| **Click the pill** | Opens the notch panel |
| **Move mouse away** | Closes automatically after a brief delay |
| **Pin icon / `Ctrl+Alt+N`** | Pins panel open until collapsed |
| **Chevron / `Esc`** | Collapses the panel |
| **`Ctrl+Alt+T`** | **Quick add**: Opens notch directly to the to-do input |
| **Scroll down over pill** | Opens the panel |
| **Scroll up over idle pill** | Cycles time, date, tasks left, focus minutes |
| **Scroll over tab row** | Switches tabs smoothly |
| **Drag file onto notch** | Opens the **Shelf** tab and parks the file |
| **`Ctrl+Alt+H`** | Tucks the notch away completely |

---

## Panel Tabs

- **Media**: Track title, artist, album artwork, scrubbable seekbar, and playback controls. Hooks directly into Windows System Media Transport Controls (Spotify, YouTube, Apple Music, browsers, VLC).
- **Controls**: Quick access to Snipping Tool screenshot and common Windows settings (Display, Sound, Network, Power & Battery, All Settings).
- **Today**: Focus timer (Pomodoro 5/15/25/50m or custom), persistent to-do checklist, and interactive month calendar.
- **Work**: ERPNext Sprint Backlog integration. Track active sprint tasks, priorities, update statuses, reassign teammates, or jump directly to ERPNext in browser.
- **Stats**: Live animated CPU and RAM meters with history graphs, battery percentage, and system uptime.
- **Clipboard**: Last 20 copied text snippets with search filtering. Star items to permanently pin them.
- **Shelf**: Drag & drop staging zone for files and folders. Keeps items handy across workspaces.
- **Apps**: Application launcher featuring real Windows icons and the native **ArtCraft** creative suite (Craft Launcher, GridCraft, DeckCraft, WordCraft).
- **Games**: Four mini arcade games (Super Flight, Brick Breaker, Slicer, Viking Run).
- **Power**: Fast access to Lock, Sleep, Restart, and Shut Down with safety confirmation.

---

## Files & Settings

All configuration and data reside in `%APPDATA%\Vey\` (access via Tray → *Open settings folder*):
- `config.ini`: User preferences, hotkeys, calendar links, and visual toggles.
- `apps.txt`: Pinned app launcher entries.
- `todos.txt`: Focus to-do items.
- `pinned_clips.txt`: Starred clipboard snippets.
- `shelf.txt`: Staged shelf file paths.

---

## Performance & Light Footprint

| Metric | Vey |
| :--- | :--- |
| **Idle CPU** | **0.0%** (zero polling, driven purely by OS event hooks) |
| **Idle RAM** | **~32 MB** private working set |
| **Binary Size** | **~1 MB** single portable executable |
| **UI Framework** | Pure Direct2D / Win32 (No Chromium, No WebView2, No Electron) |

---

## Contributing

Contributions, feature requests, and bug reports are welcome! Please check out [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Vey is licensed under the [MIT License](LICENSE).
Portions derived from [Nock](https://github.com/AkhileshVarute/Nock) © Akhilesh Varute.
