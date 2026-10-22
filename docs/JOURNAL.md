> **Historical development journal.** This file records the research and diagnostics, in order. Some passages are outdated (the "spikes" have been removed, the `C:\tmp\lol` paths no longer exist, `mod-tools.exe` is no longer used). The current state of the project is in the [README](../README.md).

# Purple — research notes

Goal: a personal LoL skin changer that does not depend on a third-party app that may stop.
References studied: Rose (Alban1911/Rose), LTK Manager + league-mod (LeagueToolkit).

## How it works (3 layers)

1. **Skin files** — a skin = a small `.fantome` (zip) holding `WAD/<Champion>.wad.client`.
   E.g. `LeagueSkins/skins/1/1001/1001.fantome` (Goth Annie) = 5 KB: a `skin0.bin` (PROP format)
   that *redirects* to `DATA/Characters/Annie/Skins/Skin1.bin`, + a bin for the sub-characters
   (Tibbers). The real assets (models, textures) are already in the game install.
   → we can **generate them ourselves** later; meanwhile we use LeagueSkins.
2. **Overlay** — copy of the game's WADs with the mod's chunks. Built by `mod-tools.exe mkoverlay`
   (cslol, spike) or by the Rust crate `ltk_overlay` (final target). ~2 s for one skin.
3. **LTK Patcher** — `ltk_patcher_host.exe` + `ltk_patcher_dll.dll` (closed binaries, maintained by
   LeagueToolkit, permissive license: reusable/redistributable, re-sign if redistributed outside the
   official release). The host injects the DLL into the game at launch; the DLL makes the game read
   the overlay instead of the official WADs (and bypasses the signature check).

## Host protocol (stdin/stdout, text)

```
config loglevel 16
config flags 4            # 4 = OPT_OUT_AH_V1: disables the anti-skinhack scan (required for us)
config prefix <overlay>\  # overlay folder, with a trailing backslash
start scan                # then launch the game AFTERWARDS (the DLL only patches games started later)
stop
```
Events: `status <ts> injecting|injected|waiting|exited|failed ...`, `error ...`, `dll <ts> <pid> <tid> <level> <msg>`.
Checked locally: the host answers `ok` to the 3 configs, then `status ... injecting scanning for game`.

## Risks / dependencies

- The DLL has an **end-of-life date** (`end of life reached, please update`): it refuses game builds
  that are too recent → new LTK binaries are needed after each patch. Plan an automatic update from
  the LTK Manager releases + patcher behind a replaceable interface.
- The DLL's anti-skinhack scan blocks skins applied on the base champion (`c0000229`)
  → flag 4 (like Rose).
- Rose suspends the game process while building the overlay; not needed if the overlay is ready
  before the game starts.
- Possible telemetry in the LTK binaries: not checked.

## Development environment (this machine)

- WSL does not start (WSL1 not supported) — not needed, Rust is installed on Windows.
- Rust MSVC installed but **no C++ Build Tools** (VS Code only) → no linker.
  Chosen solution (2026-10-03): **w64devkit 2.10.0** (gcc 16.2) in `C:\Users\pierr\tools\w64devkit`
  + Rust toolchain `stable-x86_64-pc-windows-gnu`, pinned by `rust-toolchain.toml`, paths in
  `.cargo/config.toml`. Nothing in the system PATH. Two patches: an empty `lib/libgcc_eh.a` added to
  w64devkit (Rust asks for it, w64devkit has none) and `-C dlltool=` in `.cargo/config.toml`.
  Tested: the `zstd` crate (C code) compiles. If the w64devkit folder is deleted: redo the patches.
- The spike's `cslol-dll.dll` stub was linked by hand with `rust-lld` (no_std, no imports).

## Technical choices

- Language: **Rust** (native `ltk_*` crates, tiny binary, minimal RAM).
- UI: light native (egui or Slint) rather than Tauri/WebView2, for RAM.
- Target pipeline: LCU websocket (champ select) → skin choice UI → `ltk_overlay` → patcher host.

## Spikes (`spikes/`)

- `setup.sh`: copies `mod-tools.exe`, `ltk_patcher_host.exe`, `ltk_patcher_dll.dll` into `spikes/tools/`
  (sources: local clones of Rose and ltk-manager in `C:\tmp\lol`). Git-ignored.
- `spike1.py <file.fantome>`: unpack → mkoverlay → host → waits for a game to start.
- `cslol_stub/`: empty `cslol-dll.dll` that `mod-tools.exe` needs to start.

## Spike 1 result (2026-10-03) — partial failure, instructive

- Injection OK: host → `game found` → `dll attached` → `overlay verified 1 wad(s)` → `redirected wad: Annie.wad.client`.
- Then the game dies ~1 s later: `FATAL ERROR - Installation is corrupt. WadFile mount failed.
  DATA/FINAL/Champions/Annie.wad.client — Problem: Corrupt`, and it writes `SOFT_REPAIR` (the client
  then starts its official repair, hence the "problem with your installation" popup).
  The game files are not touched (the overlay is in a separate folder).
- Likely cause (NOT confirmed): `mod-tools.exe mkoverlay` (cslol, old) rewrites the WAD without
  handling WAD v3.4 "zstd + sub-chunks" chunks (type 4) correctly: the overlay's sub-chunk indices
  differ from the original for 1562 chunks out of 4083. The current game rejects the result.
- To test: build the overlay with the modern `ltk_overlay` crate (what LTK Manager does)
  → needs a compiler (MSVC Build Tools or MinGW); or use LTK Manager as an oracle.
- The anti-skinhack scan fails as expected (`c0000229`, "opted out" thanks to flag 4): OK.

## Spike 2 result (2026-10-03) — SUCCESS

- `spikes/overlay` (Rust, crate `ltk_overlay` 0.16.2 + `FantomeContent`) builds the overlay in ~0.6 s.
  Only 2 chunks differ from the original (against 1562 with mod-tools.exe): the cause of spike 1's crash
  was indeed the WAD rewrite by the old cslol.
- In-game test: Goth Annie shown on the base skin (Practice Tool). Pipeline validated:
  LeagueSkins `.fantome` → `ltk_overlay` → `ltk_patcher_host` (flags=4) → game.
- `mod-tools.exe` and the `cslol-dll.dll` stub are no longer needed (spike 1 obsolete).
- `spikes/host.py <overlay_dir>` only starts the host on an overlay already built.

## Next steps
1. `purple` binary (Rust): LCU (lockfile + websocket) → detects champ select / champion / wanted skin.
2. Download/cache the LeagueSkins `.fantome` files (index by champion/skin, chromas).
3. Drive the host from Rust (spawn, text protocol, events), start the scan before the game starts.
4. Light native UI (egui/Slint) + tray; automatic update of the patcher host+DLL pair (end-of-life date).
5. Later: generate the skins locally instead of depending on LeagueSkins.

## `purple` binary (crates/purple) — state on 2026-10-03

Commands: `status`, `skins <champion>`, `set <champion> <skin>`, `unset`, `list`, `run`, `build <champ id> <skin id>`.
`run`: polls the client (gameflow-phase + champ select); when the champion is picked → downloads the .fantome
(LeagueSkins, cache `%LOCALAPPDATA%\Purple\skins`) → `ltk_overlay` → patcher host (`start scan`); never stops
the patcher while a game is attached. Env: `PURPLE_DEBUG=1` (full logs), `PURPLE_DATA_DIR`,
`PURPLE_PATCHER_DIR`. Patcher binaries: `vendor/ltk-patcher/` (license included).

### Fixed bug: "overlay verified 0 wad(s)" (no skin visible) — CONFIRMED in game
- Symptom: injection OK, but the DLL sees no WAD in the overlay → default skin.
- Cause: the overlay was under `%LOCALAPPDATA%`, so under `C:\Users\<name>`, whose ACL only allows the
  account, SYSTEM and administrators. The game cannot traverse that folder. Granting read access to the
  final folder only (`icacls` test on `overlay\`) changes NOTHING: incomplete diagnosis, refuted in game.
  Same overlay, byte for byte, in `C:\tmp\pd` or `C:\ProgramData\Purple` → it works.
- Fix: overlays live in `%PROGRAMDATA%\Purple` (readable by the Users group, inherited by the files).
  Skin cache and config stay in `%LOCALAPPDATA%\Purple` (the game does not read them).
- Validated in game on 2026-10-03: Goth Annie (1001) and Primordian Aatrox (266033), via `purple run`,
  default paths. Logs: `AH wad scan failed c0000229 ... (opted out)` → `overlay verified 1 wad(s)` →
  `redirected wad: .../<Champion>.wad.client`.
- `run` shows an explicit warning if the DLL keeps reporting `overlay verified 0 wad(s)`.
- Measured: `purple` ~15 MB of RAM + `ltk_patcher_host` ~8 MB; release binary 3.3 MB.

## Window (`purple.exe`) — 2026-10-03

- Structure: `purple` lib (catalog, config, engine, gui, images, lcu, overlay, patcher, store) + 2 binaries:
  `purple` (window, Windows subsystem in release, no console) and `purplectl` (command line).
- `engine::run` = shared loop (`engine::Event` events); the window runs it in a thread, `purplectl run`
  prints it in the terminal. The window writes `config.json`, the engine reads it again every second of champ
  select (changing the skin during champ select re-arms).
- UI: egui/eframe 0.36 (`glow` backend, not `wgpu`: lighter). 0.36 API: `App::logic` + `App::ui`, `egui::Panel`.
- Pictures: portraits + skin tiles come from the client (`/lol-game-data/assets/...`), disk cache
  `%LOCALAPPDATA%\Purple\img`, decoded/downscaled in a thread, textures capped (240). Catalog cached
  (`catalog.json`): the window opens even with the client closed.
- `Jade_*` champions (ids >= 60000, another game mode) hidden: they duplicated every name.
- The window refuses to close during a game (killing the patcher under the game = risk).
- Checked with captures + simulated clicks: choice, cancel, "default skin" button, config.json updated.
- Release memory: window open ~115 MB (mostly OpenGL context + fonts + textures), against ~15 MB for
  the engine alone. Idea: tray daemon (engine only, ~15 MB) + window started on demand in a separate process.
- Screenshots on Windows: `SetProcessDPIAware` is required, otherwise the capture is cropped (125 % scale).

## Tray (2026-10-03) — final architecture with 3 binaries

- `purple.exe`: resident program (engine + notification-area icon, tray-icon 0.26 + Win32 message
  loop). Release: ~15 MB (3.6 MB private), no window. Menu: state (greyed out), Open Purple, Quit.
  Left click on the icon = opens the window. "Quit" during a game = waits for the game to end.
  Single instance (mutex `Local\Purple.Daemon`); a 2nd launch opens the window. Log: `purple.log`.
- `purple-ui.exe`: the window (egui). Separate process, started on demand, frees its ~100 MB when closed.
  Single instance (`Local\Purple.Window`, otherwise focuses the existing window, title "Purple Skin Changer").
  Read-only state: `status.json` (written by the tray), writes `config.json`. Shows "Engine stopped" +
  a "Start it" button if the tray is not running.
- `purplectl.exe`: command line (dev/diagnostics); do not run `purplectl run` alongside the tray.
- The patcher is attached to a KILL_ON_JOB_CLOSE Windows job: it dies with `purple.exe` (crash, kill) instead of
  staying active and patching a later game with an old overlay.
- The patcher is started with CREATE_NO_WINDOW (otherwise a black console shows when the parent has no console).
- Windows 11 puts new icons in the "^" menu (hidden icons): drag it to the taskbar.
- Still to do: start with Windows, icon in the .exe (resource), English skin names
  (CommunityDragon en_gb has the same ids: 2159 skins; names currently in the client's language),
  automatic update of the LTK patcher (end-of-life date), chromas, skins already owned.
