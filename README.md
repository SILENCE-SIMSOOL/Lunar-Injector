# Lunar Injector
<img width="1024" height="576" alt="image" src="https://github.com/user-attachments/assets/c1627d1b-1427-4088-a330-c66028cca2cf" />

Lunar Client 1.8.9 injector and utility tool built with Rust and egui.

---

## Features

- **Modern Dark UI**: Clean, responsive interface embedded with the official Inter font.
- **Process Detection**: Automatic background scanning for active Lunar Client 1.8.9 processes.
- **HitDelayFix**: Removes the 1.8.9 attack delay reset when missing a swing.
- **AutoClicker**:
  - **Range Mode**: Configurable minimum and maximum CPS thresholds.
  - **Graph Mode**: Customizable 11-step probability curve (10–20 CPS) normalized to 100%.
- **Conditions & Trigger Filters**:
  - Disable clicking when aiming at a block.
  - Optional keybind-based activation.
  - Item-specific white-listing (`Hand`, `Sword`, `Axe`, `Fish`, `Stick`).
- **Zero External Runtime Dependencies**: Embedded assets and single installer executable.

---

## Installation

### Installer
Download and run `LunarInjector_Setup.exe` from the latest release. The setup wizard will handle desktop shortcuts and install required runtime components (`LunarInjector.exe` and `lunar_bridge.dll`).

### Portable
Extract `LunarInjector.exe` and `lunar_bridge.dll` into the same directory and execute `LunarInjector.exe`.

---

## Building from Source

### Prerequisites
- [Rust](https://www.rust-lang.org/) (MSVC x64 toolchain)
- [Inno Setup 6](https://jrsoftware.org/isinfo.php) (optional, for compiling the setup installer)

### Build Commands
Run the automated build script:
```powershell
powershell -ExecutionPolicy Bypass -File scripts/build.ps1
```

Or build manually via Cargo:
```powershell
cargo build --workspace --release --target x86_64-pc-windows-msvc
```

The compiled binaries will be output to `dist/`.

---

## License

This project is released under the MIT License.
