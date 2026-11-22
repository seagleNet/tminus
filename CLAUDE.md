# tminus

A terminal countdown timer styled after Omarchy's screensaver: a big clock in the
theme's colors with animated text effects. Rust, crossterm, clap. The package is
`tminus`; the **binary is `t-`** (`[[bin]]` in Cargo.toml). Repo:
https://github.com/seagleNet/tminus (default branch `master`).

## Commands

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings   # pedantic is on (Cargo.toml [lints])
cargo test
cargo +1.88 test                                     # MSRV; keep in sync with rust-version and ci.yml
cargo test measure_durations -- --ignored --nocapture  # worst-case effect durations
cargo install --path .                               # installs ~/.cargo/bin/t-
```

CI (`.github/workflows/ci.yml`) runs fmt, clippy, tests on Linux + macOS and the
MSRV job; it must stay green. `release.yml` runs on `v*` tags.

## Layout

- `main.rs`: startup, config merge, subcommand dispatch, TUI loop (`run`), headless loop, alerts at zero (`announce`)
- `cli.rs`: clap options/subcommands, `KEYS` (popup keybindings), duration parsing and clock formatting
- `app.rs`: `App`, i.e. countdown and sequence logic (`tick`, keys, `remote`), effect scheduling, drawing, status JSON
- `plan.rs`: sequence tokens (`25m:work@nord`, `x4`, `@preset`)
- `clock.rs`: `Timer` (suspend-aware `BootInstant`; wall-clock mode for `--until`), `--until` parsing incl. DST
- `config.rs`: `~/.config/tminus/config.toml` turned into CLI args (plus `[themes.*]`, `[presets]`)
- `control.rs`: Unix-socket remote control (`t- ctl status [--follow]`, `t- ctl list`, `t- ctl attach [--take]`, `t- ctl <command>`)
- `effects/`: `mod.rs` (Kind enum, pools, `make`, `max_duration`, `measure`, shared helpers), `motion.rs` (effects where characters travel, many through `Travel`) and `light.rs` (effects in place); all ported from TerminalTextEffects
- `scene.rs` (clock canvas and settled look), `term.rs` (frame buffer and diffing presenter), `font.rs`, `theme.rs`, `util.rs`
- `nixie.rs`: `--font nixie` (wire digits after the IN-8-2, glass, unlit cathodes, glow; `Style` from the nixie options)

## Design rules (non-obvious; keep them)

- **Config is merged as CLI args** placed *before* the real ones; `args_override_self` makes the later (command-line) value win. Every on/off flag therefore has a hidden opposite (`--no-bar`, `--bell`, ...) via `overrides_with`. Config values go as one `--key=value` argument (a value may start with `-`). List options use `ArgAction::Set` so a CLI `--effect` replaces the config's list instead of appending.
- **Never let crossterm's event reader see a hung-up terminal**: it spins at 100% CPU forever (crossterm-rs/crossterm#793). Input goes through `Term::wait_input` (own `poll(2)` on `/dev/tty`); `event::read` is only called after it reports `Ready`. Resizes are detected by polling `terminal::size()`.
- **Effects must not hide the clock for long**: each `Kind` has a `max_duration` (checked by `effects_fit_their_durations` on a normal and a large layout, which must stay ≤ 5s except matrix). The final-seconds window (`--alert`, else 10s) is effect-free; effects only start if they finish before it (`effect_choices`). Per-row staggers use `row_gap()` so tall block-font clocks don't stretch effects.
- **`--effect-align`**: the effect is measured off-screen (`effects::measure`, same seed via `make_seeded`) and started so it ends exactly on a round `left` value; after that, marks step by `--every` rounded up to the alignment (`mark_after`).
- Effects read the *current* target chars each frame (the clock keeps ticking mid-effect); ordering is computed over non-space cells at construction.
- Timing: `Timer` (`total`, `elapsed()`) is sampled once per frame (`App::sample`); animation time is `f64` since start. The displayed time rounds *up*.
- **Nixie font**: each cell is `▀` with the upper pixel as fg and the lower as `Cell::bg` (octants fitted to the colors, 2x4 with two colors a cell, were tried and dropped: a cell often needs three colors, so they speckled glow, cut into glass and wires, and shimmered as pulses changed the colors; small tubes, under `SMALL_TUBE_W`, do use octants (`--small-tubes`, auto in Ghostty/Alacritty/foot/kitty), but picked from the shapes alone: each pixel is lit wire or nothing (`fine_cells`), so colors (pulse, flash) never change the pattern; no glass (sharing a cell with a wire it needs a third color: it got cut, and keeping wires out of its cells left letters too small), and no glow or unlit symbols (tried at cell resolution: brown boxes and dark blobs), the symbols fill the tube; they go down to `MIN_FINE_TUBE_W`), so the tubes are a true-color image blended over `Scene::rest` (the fill, else the terminal's queried background). `Target.chars` hold only the wires as `▀▄█` (owner per digit), which is what effects animate and `settled` returns; `nixie::draw` paints the full image at rest, and only glass and unlit cathodes (`lit = false`) under an effect, which then lights them like a bloom pass each frame (`nixie::under_effect`, `bloom`, `light`: what it drew over the unlit tubes is read back as pixels and blurred with the glow's blur, plus the rim and streak light (`halo` at `NEAR_R`); wide blurs at reduced resolution, `soft`; separators, having no glass, get no bloom but fade in with their dots, `separator`; cells the effect has settled, `settled_cells`, show exactly as at rest, so it ends on the tubes' own wires; the tube cache keeps lit and unlit apart, both being drawn each frame), and after it the whole screen crosses over from its real last frame (copied by `draw_canvas` before the footer is drawn, kept in `App::fx_end`; it ends on settled wires in `Scene::lit_wire_color`) to rest (`nixie::crossfade`, `REST_FADE`; its symbols fade out over the first half and get a background only where both pixels are glass, `behind`; a pixel drawn on one side only stays empty until it shows, or it would be an opaque patch in a transparent terminal). Distances and each cathode's per-pixel shading (`Shade`) are computed once per tube size (`Field::get`; the glow is the wire blurred like a bloom pass, two Gaussians fitted to `exp(-d / GLOW_R)`, added in linear light via tables in `glow_onto`; wires are at least a pixel wide), and with neon colors each tube's cells are cached until what's lit, its colors or the background change (`SlotKey`): that keeps it at block-font CPU even on huge small-font terminals (`--nixie-color theme` holds the gradient still for this; `theme-heavy` drifts like the other fonts and redraws every tube each frame). `Frame::put` keeps a cell's background so effect chars sit on the glass; popups clear cells with `Frame::set(.., None)` first. Each tube holds a `Group` of up to 11 cathodes like a real one (digits + comma, `A-K`, `L-V`, `W-Z`, units `dhm` after a digit, signs), each group's shading worked out per tube size on first use (`Field::cathodes`); letters are uppercase IN-15-style wires (`symbol_strokes`). The large title is a second `Layout` (`nixie_title`, `dy` below the clock) in tubes sized off the screen width (`nixie::title`, else block letters); the small one is plain text in the tube color. A character no tube has gets a blank tube (`Lamp::Blank`); a done-text may wrap at spaces onto up to 4 lines when that gives bigger tubes (`wrappings`, `Slot::y`; the clock never wraps). Only when even the smallest tubes don't fit does it fall back to the other fonts, which keep the tube colors (`Target::nixie_look`, used by `Scene::base_color`). The digit tick is a crossfade (`Slot::prev`) in `FADE_STEPS` steps (frames between them are identical, so cached and not resent), not the scramble. With nixie tubes the `--alert` pulse flares the tubes (`Scene::pulse`) instead of flashing the background, which would rewrite the whole screen each frame. On a light background (`Colors::light`) glass and unlit cathodes are darker than it, glow tints instead of adding light, and the core deepens instead of going white.
- Rendering: colors within ±3 per channel count as unchanged when diffing (big output savings); wide chars take two cells (`WIDE_TAIL`). Idle frames drop to 20 fps unless something animates.
- Spawned helpers (notify-send, `--sound`, `--exec`) are started immediately and only reaped on a thread, so they survive `--exit`.
- Themes: built-ins are compile-time checked (`Rgb::hex`); all installed Omarchy themes (`$OMARCHY_PATH/themes`, `~/.config/omarchy/themes`, plus the active one in `$XDG_STATE_HOME/omarchy/current/theme` with `theme.name`; older Omarchy: `~/.config/omarchy/current`) are read from `colors.toml` (named keys: `red`, `blue`, `muted`, ...) / `alacritty.toml` / `kitty.conf` and replace same-named built-ins; config themes override all.
- **Flashes fade to the terminal's real background**, queried at startup (`term::query_background`: OSC 11 plus a DA1 sentinel so non-supporting terminals don't stall, 300ms cap); the theme's `bg` is only the fallback. Skipped inside Neovim/Vim terminals (`$NVIM`, `$VIM_TERMINAL`): libvterm reports black, not what is drawn. Fading to a guessed color makes the last frame jump when the fill is dropped.
- **`--demo`** (`app::Demo`) reuses the normal app: `effect_choices` returns the chosen kind, `next_replay` loops it, `f` swaps in a short `Timer` and `tick` restores the full one after the zero flash. No announce/exec/socket in demo; `run` takes `&mut App` so `main` can print the choice after `term::restore`.
- **Completions are dynamic** (`clap_complete` `unstable-dynamic`, pinned `~4.6`): `CompleteEnv` at the top of `main` answers `COMPLETE=<shell> t-`; `t- completions <shell>` prints the same hook (zsh gets a `funcstack` tail so it works autoloaded from `$fpath`). The static scripts broke on positional DURATION + subcommands. `--theme`, `ctl -n` and DURATION have custom candidates (`cli::theme_candidates`, `cli::timer_candidates`, which asks the running timers like `ctl list`, and `cli::complete_duration`: `@preset` via `plan::with_builtins`, `25m@theme`); the `completions` test guards the engine.
- **`t- ctl` words are subcommands** (`CtlCommand`, `--name` is global) for help and completion; `CtlCommand::line` turns them into the socket protocol's text line (`App::remote`), checking durations first. `--demo`/`--list-*` stay flags on purpose: subcommands don't get the config merged in. `status` and `list` live under `ctl` too (0.1/0.2 had a top-level `t- status`; dropped without an alias since nobody used it yet); `list` asks each socket for `info` (tab-separated, see `App::info_line`) and shows older timers that don't know it as `?`.
- **Timer names are unique** (so `t- ctl -n` reaches each one): an explicit `--name` in use by a live timer is refused; unnamed timers get `control::free_name` (`main::timer_name`): the first `@preset`'s name, else a `-t` title given on the command line (not the config's `title`, which would override its `name`; slugged by `control::slug`), else the config's `name` (taken out of the config like `[presets]`, not passed as `--name`, so it can't block a second timer), else/then Docker-style adjective + noun (`eager-comet`, `eager-pomodoro`). Dead sockets don't count. The status JSON has the name (field + last tooltip line); `ctl status --follow` without `-n` re-checks for a newer timer about once a second and moves on without an idle line while another timer runs.
- **`t- ctl attach`**: the timer answers `state` with `App::handover_state` (segment, total, elapsed, a `CLOCK_BOOTTIME` timestamp, paused, synced) plus its own argv, hex-encoded, minus `--headless` (`control::handover_reply`). `attach` re-execs `t-` with that argv (config read again) and `--resume-state`: as a viewer (`--view NAME`: no socket, no announce, keys that change the timer go to it via `App::remote_line_for`, `App::sync` every `VIEW_SYNC`, segment changes come from the timer), or with `--take` (`handover`: a headless timer replies and quits, the new process takes its `--name`). A finished segment resumes as done without a second announcement. `--take` first runs the new command line with `--check` (as a viewer, the name is still taken; `main` checks `--resume-state` before `--check` returns) and only then asks for `handover`, so a changed config can't lose the timer. Resuming never re-reads `--until` (it may name another day by now): a synced snapshot becomes a wall-clock timer again (`Timer::resumed`). `handover_reply` drops `--view`/`--resume-state` from the argv, or the stale ones would override the new.
- **Errors without a terminal** (keybinding, bar click: none of stdin/stdout/stderr is a tty) also go to `notify-send` (`main::report`, also for clap's own errors via `try_parse_from`). `--check` runs everything a start would check (options, config, plan, themes, sound file, a taken `--name`) and exits before the socket; it never notifies (`main::check_only` scans the raw args, so clap errors are covered too). The Omarchy widget runs it before starting a timer, to show errors under its input field.
- Socket path: `$XDG_RUNTIME_DIR/tminus/<name>.<pid>.sock`, falling back to `/tmp/tminus-<uid>` when that path would exceed the ~108-byte socket limit. The directory must be a real directory owned by us (`control::check_owner`), else no socket is used: in `/tmp` another user could plant sockets that `attach` would take a command line from.

## Effects and attribution

All 37 effects (in `effects/motion.rs` / `effects/light.rs`) are adaptations of
TerminalTextEffects (MIT, © ChrisBuilds) via its Rust port ttfx (MIT, © 37signals),
which Omarchy's screensaver uses. Keep the notices in `LICENSE` and the README
credits. They are re-implementations of the documented designs (palettes, symbols,
phases), shortened, not frame-exact ports. Names match tte's (`orbittingvolley`,
`randomsequence`, ...). New effects need a `max_duration` entry and must pass
`every_effect_finishes` and `effects_fit_their_durations`.
The exception is `slotmachine` (`nixie.rs`, tminus's own): nixie-only (left out
of `effect_choices` for other fonts, ends at once there), and it draws the whole
tubes itself (`Effect::keeps_tubes`), so no backdrop and no glow fade after it.

## Testing the TUI without a terminal

Run `t-` in a pty and render it with pyte (`uv run --with pyte python ...`):
`pty.fork()`, set the size with `TIOCSWINSZ` and SIGWINCH, feed the output into
`pyte.ByteStream`. Keep draining the pty (e.g. from a thread) while running other
commands, or `t-` blocks writing and stops answering. Pass `--no-config` for
reproducible runs. Answer `ESC [ c` with e.g. `ESC [ ? 62 c` (and optionally
`ESC ] 11 ; ?` with `ESC ] 11 ; rgb:0c0c/0c0c/0c0c ESC \`), or startup waits 300ms. Use a *short* `XDG_RUNTIME_DIR` for socket tests. Color-only
effects (highlight, colorshift, spotlights) are invisible in text snapshots.

## Release status

- **Released:** 0.1.0, 0.2.0, 0.3.0 (all 2026-11-12), 0.3.1 and 0.4.0 (2026-11-14), 0.5.0 (2026-11-21), 0.6.0 (2026-11-22): public repo, tags with GitHub release
  binaries (`release.yml`), published on crates.io (`cargo install tminus`). The README GIFs
  were re-recorded for 0.6.0 (2026-11-22, the reworked effects).
- Next release: bump `version` in Cargo.toml, tag `vX.Y.Z` (the push runs `release.yml`),
  `cargo publish` from the tagged commit; bump the GIF URL tag in the README if re-recorded. `cargo publish`'s verify step builds the packaged copy into the shared `target/` and leaves `target/debug/t-` pointing at `target/package/` sources (edits then look "fresh"): run `cargo clean -p tminus` afterwards.
- AUR: not submitted yet (registrations were closed). When it opens: `updpkgsums` +
  `makepkg --printsrcinfo > .SRCINFO` in `packaging/aur/`.
- Never tested for real: Waybar itself (current Omarchy has omarchy-shell instead; `t- ctl status
  --follow` was checked through a pipe), the Hyprland binds in the README, running on macOS
  (CI only builds and tests it).
- README GIFs: `vhs/render.sh` (tapes in `vhs/`, output `assets/`, needs vhs + gifsicle). Tapes use a fresh `HOME`, no Omarchy themes, and hidden aliases for flags; re-run after visible changes. The README links them by absolute URL pinned to the full hash of the commit that added them (`raw.githubusercontent.com/.../<sha>/assets/...`, so the crates.io page works and GitHub shows them at once); after re-recording, commit the GIFs, then point the URLs at that commit. Never rewrite `master` history past it.
- Ideas not done: CHANGELOG.
