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
- `control.rs`: Unix-socket remote control (`t- status [--follow]`, `t- ctl ...`)
- `effects/`: `mod.rs` (Kind enum, pools, `make`, `max_duration`, `measure`, original effects), `motion.rs` and `light.rs` (effects ported from TerminalTextEffects)
- `scene.rs` (clock canvas and settled look), `term.rs` (frame buffer and diffing presenter), `font.rs`, `theme.rs`, `util.rs`

## Design rules (non-obvious; keep them)

- **Config is merged as CLI args** placed *before* the real ones; `args_override_self` makes the later (command-line) value win. Every on/off flag therefore has a hidden opposite (`--no-bar`, `--bell`, ...) via `overrides_with`. List options use `ArgAction::Set` so a CLI `--effect` replaces the config's list instead of appending.
- **Never let crossterm's event reader see a hung-up terminal**: it spins at 100% CPU forever (crossterm-rs/crossterm#793). Input goes through `Term::wait_input` (own `poll(2)` on `/dev/tty`); `event::read` is only called after it reports `Ready`. Resizes are detected by polling `terminal::size()`.
- **Effects must not hide the clock for long**: each `Kind` has a `max_duration` (checked by `effects_fit_their_durations` on a normal and a large layout, which must stay ≤ 5s except matrix). The final-seconds window (`--alert`, else 10s) is effect-free; effects only start if they finish before it (`effect_choices`). Per-row staggers use `row_gap()` so tall block-font clocks don't stretch effects.
- **`--effect-align`**: the effect is measured off-screen (`effects::measure`, same seed via `make_seeded`) and started so it ends exactly on a round `left` value; after that, marks step by `--every` rounded up to the alignment (`mark_after`).
- Effects read the *current* target chars each frame (the clock keeps ticking mid-effect); ordering is computed over non-space cells at construction.
- Timing: `Timer` (`total`, `elapsed()`) is sampled once per frame (`App::sample`); animation time is `f64` since start. The displayed time rounds *up*.
- Rendering: colors within ±3 per channel count as unchanged when diffing (big output savings); wide chars take two cells (`WIDE_TAIL`). Idle frames drop to 20 fps unless something animates.
- Spawned helpers (notify-send, `--sound`, `--exec`) are started immediately and only reaped on a thread, so they survive `--exit`.
- Themes: built-ins are compile-time checked (`Rgb::hex`); the active Omarchy theme's colors are read from its `colors.toml` / `alacritty.toml` / `kitty.conf`; config themes override both.
- Socket path: `$XDG_RUNTIME_DIR/tminus/<name>.<pid>.sock`, falling back to `/tmp/tminus-<uid>` when that path would exceed the ~108-byte socket limit.

## Effects and attribution

The 26 effects in `effects/motion.rs` / `effects/light.rs` are adaptations of
TerminalTextEffects (MIT, © ChrisBuilds) via its Rust port ttfx (MIT, © 37signals),
which Omarchy's screensaver uses. Keep the notices in `LICENSE` and the README
credits. They are re-implementations of the documented designs (palettes, symbols,
phases), shortened, not frame-exact ports. Names match tte's (`orbittingvolley`,
`randomsequence`, ...). New effects need a `max_duration` entry and must pass
`every_effect_finishes` and `effects_fit_their_durations`.

## Testing the TUI without a terminal

Run `t-` in a pty and render it with pyte (`uv run --with pyte python ...`):
`pty.fork()`, set the size with `TIOCSWINSZ` and SIGWINCH, feed the output into
`pyte.ByteStream`. Keep draining the pty (e.g. from a thread) while running other
commands, or `t-` blocks writing and stops answering. Pass `--no-config` for
reproducible runs. Use a *short* `XDG_RUNTIME_DIR` for socket tests. Color-only
effects (highlight, colorshift, spotlights) are invisible in text snapshots.

## Release status

- Private repo, CI green on Linux, macOS and MSRV. Not yet public, no tag yet.
- Release: `gh repo edit seagleNet/tminus --visibility public --accept-visibility-change-consequences`,
  then `git tag v0.1.0 && git push origin v0.1.0` (`release.yml` has never run yet),
  then for the AUR `updpkgsums` + `makepkg --printsrcinfo > .SRCINFO` in `packaging/aur/`.
- Never tested for real: actual Omarchy theme files (format assumed), Waybar/Hyprland
  examples in the README, effects in a real color terminal at full speed, zsh
  completions, running on macOS (CI only builds and tests it).
- Ideas not done: README demo recording (vhs), CHANGELOG.
