#!/usr/bin/env bash
# Re-record the README GIFs: needs vhs (with ttyd, ffmpeg) and gifsicle.
# Run from anywhere; tapes run one at a time so the recordings don't stutter.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --quiet
for tape in hero demo alert; do
    vhs "vhs/$tape.tape"
    gifsicle -O3 --lossy=60 --colors 128 --batch "assets/$tape.gif"
done
ls -lh assets/*.gif
