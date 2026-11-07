# Lunar Injector

Rust-only Windows x64 application for Lunar Client / Minecraft 1.8.9.

## Features

- Single-page monochrome dark GUI matching the reference layout: custom window title bar, full-size controls, fixed 11-column CPS graph.
- Detects Minecraft windows belonging to `javaw.exe`/`java.exe` and auto-selects a discovered process, refreshing every two seconds.
- Loads a Rust DLL into the selected Java process; the DLL attaches to JNI/JVMTI.
- HitDelayFix: resets `leftClickCounter` only while looking at `MISS`, in game and outside menus.
- AutoClicker: left click while the physical left button is held; right click while the configured hotkey is held.
- Optional no-click-on-block safeguard.
- Allowed item types: Hand, Sword, Axe, Fishing Rod, Stick (identified through 1.8.9 vanilla item IDs).
- Range mode: minimum/maximum CPS from 10 to 20.
- Graph mode: 11 separate sliders and numeric inputs, one for each CPS from 10 through 20. Changing one percentage redistributes the other ten, keeping the total exactly 100%.
- Config is persisted at `%APPDATA%\LunarInjector\settings.json`.

## Build

Requires Windows 10/11 x64, an up-to-date Rust MSVC toolchain, and the Visual Studio C++ Build Tools (MSVC and Windows SDK). **No C++, C#, .NET, or JDK headers are used in the source.**

```bat
build.bat
```

`build.bat` checks for Rust/rustup, installs the Windows x64 MSVC target if needed, runs the shared tests, builds the release EXE and DLL, and copies both to `dist`. Visual Studio C++ Build Tools (MSVC linker and Windows SDK) must already be installed. PowerShell users can still use `scripts\build.ps1`.

Output:

```text
dist/
  LunarInjector.exe
  lunar_bridge.dll
```

## Run

1. Start Lunar Client 1.8.9 and join a world.
2. Double-click `run.bat` (or launch `dist\LunarInjector.exe` directly). `run.bat` refuses to start if the EXE or DLL is missing.
3. When the game window is detected, press **Inject**.
4. Wait for `Bridge connected · Minecraft mapping OK` while inside a world and pointing at a valid target.
   - `Injected` means the native bridge is running; it does not mean Minecraft mappings are available.
   - `Restart Client` means the DLL was loaded but no live connection was confirmed. Restart Lunar Client before trying again.
5. Change settings in the app. Updates are written to the shared configuration file.

The DLL stays loaded for the lifetime of the Java process. Close the game to unload it.

## Implementation scope and limits

- **Windows build and runtime have not been verified here.** The build toolchain is unavailable in the authoring environment. The batch scripts and dependency versions were inspected statically, but no `cargo build`, `.bat` execution, or Lunar Client integration run was possible. This remains source code, not a verified executable.
- **Lunar Client support cannot be guaranteed without an exact-build test.** Code uses documented Win32/JNI/JVMTI APIs and Aoko's public 1.8.9 mapping names. If names differ in your Lunar build, mappings fail closed and neither automatic clicking nor HitDelayFix is guaranteed to operate.
- JNI field changes currently occur from an attached worker thread, not the Minecraft game thread. Correctness and timing require in-game validation; JNI attaches successfully does not prove thread-safe gameplay integration.
- Process detection searches visible Java Minecraft windows; it cannot independently prove a detected process is Lunar Client 1.8.9. Choose the correct process before injecting.
- Mapping recognition is restricted to known Minecraft 1.8.9 class/field/method names (`Minecraft`/`ave` and named/SRG members). Unknown/mismatched mappings fail closed: autoclick does not run. This is intentional; no guesses at obfuscated field offsets are made.
- Focus/GUI checks protect menus and other apps. Unknown target/held-item state also disables automatic clicks. A low-level mouse hook tracks physical left clicks separately from injected `SendInput` clicks; if that hook cannot be installed, automated clicking stays disabled.
- The application has no anti-cheat bypass or concealment logic.
- Uses the Windows `SendInput` API. Actual mouse event processing, CPS timing and event recognition can differ by Lunar build and game settings.
- DLL initialization is verified through a status heartbeat instead of treating `LoadLibraryW` completion alone as a working bridge. Configuration and heartbeat JSON are updated using temporary files and rename.
- Right-click automation requires holding a bound key; left click requires holding the mouse button.
- No auto-updating, background service, network access, telemetry, GUI overlay or unnecessary modules.
- The HitDelayFix changes client-side attack handling; some multiplayer servers prohibit it or auto-clicking.

## Reference / license

Inspired by the publicly documented 1.8.9 JNI mapping and HitDelayFix behavior of [Aoko Client](https://github.com/naczo5/aoko-client) (GPL-3.0). Implementation is Rust, not a copy of Aoko's DLL. Distributed under GPL-3.0 to preserve compatibility with the reference project's licensing terms.

## Verification on Windows

Run `build.bat` from Explorer or Command Prompt (or `./scripts/build.ps1` in PowerShell). The script tests the shared configuration library and compiles both the GUI and the bridge for x64 Windows. To check integration, use a disposable Lunar 1.8.9 instance:

1. Confirm startup, failed injection and successful bridge handshakes show different statuses.
2. With an empty hotbar, confirm `Hand` works and all unselected item categories are blocked.
3. Verify clicks stop when the crosshair points at a block or the game loses focus; verify unknown target state never triggers clicks.
4. Hold left click and confirm only selected item categories trigger clicks. Check right click works only while the configured key is held.
5. Check both Range and Graph CPS modes; all 11 probabilities should total 100% after any edit, including 0% and 100%.
6. Change the right-click keybind, close and reopen the app, and verify it persisted.
7. Test restarting Lunar, stale status removal and incorrect mapping behavior. Do not assume compatibility if the bridge reports unresolved mappings.

These runtime steps have **not** been run in this environment.

## Batch script failure messages

- `Rust and rustup are required`: install stable Rust with the `x86_64-pc-windows-msvc` toolchain.
- `link.exe` or Windows SDK errors: install the Visual Studio **Desktop development with C++** workload, including MSVC and Windows SDK.
- `Build output is missing`: the expected `LunarInjector.exe` or `lunar_bridge.dll` was not produced.
- `Build output is missing. Run build.bat first`: the release files are not present in `dist`.
- The batch scripts only verify build prerequisites, artifact presence and process launch. They **cannot** guarantee successful JVM mapping or DLL injection.
