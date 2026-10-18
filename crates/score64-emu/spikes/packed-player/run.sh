#!/bin/sh
# E3 spike #20: pack a GoatTracker song, render it with score64-emu, VICE and sidplayfp on 6581 and 8580, and compare.
#
#   run.sh <song.sng> <workdir> [effect.bin]
#
# With an effect and KICKASS=/path/to/KickAss.jar, also assembles sfx-driver.asm, runs it in x64sc and diffs its SID
# writes against the helper's for the same effect schedule.
#
# Needs on PATH: gt2reloc (GoatTracker 2.77), sidplayfp (3.2.2 on libsidplayfp 3.1.1 with libresidfp), vsid (VICE 3.10),
# uv. Outputs land in <workdir>, which should be git-ignored (for example scratch/). See
# docs/research/2026-10-18-packed-player-spike.md.
set -eu

song=$1
work=$2
effect=${3:-}
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../.." && pwd)
seconds=60
cycles=$(( (seconds + 1) * 985248 ))

mkdir -p "$work"
work=$(cd "$work" && pwd)
name=$(basename "$song" .sng)
tune="${name}_d1h1"

cargo build --release -q -p score64-emu --manifest-path "$root/Cargo.toml"
emu="$root/target/release/score64-emu"

# Zero page $F8/$F9, clear of the common $FB-$FE pointers (docs/reference/player.md).
gt2reloc "$song" "$work/$tune.sid" -D1 -H1 -ZF8 >/dev/null
gt2reloc "$song" "$work/${name}_plain.sid" -ZF8 >/dev/null

for chip in 6581 8580; do
  "$emu" render "$work/$tune.sid" --seconds $seconds --chip $chip \
    --wav "$work/${tune}_helper_$chip.wav" --writes "$work/${tune}_helper_writes.tsv" --opcodes "$work/${tune}_opcodes.tsv"
  case $chip in 6581) m=o ;; 8580) m=n ;; esac
  sidplayfp -t$seconds -m${m}f --delay=0 -w"$work/${tune}_sidplayfp_$chip.wav" "$work/$tune.sid" >/dev/null 2>&1
done
"$emu" render "$work/${name}_plain.sid" --seconds $seconds --wav "$work/${name}_plain_helper_6581.wav" \
  --opcodes "$work/${name}_plain_opcodes.tsv"
if [ -n "$effect" ]; then
  "$emu" render "$work/$tune.sid" --seconds $seconds --wav "$work/${tune}_sfx_helper_6581.wav" \
    --opcodes "$work/${tune}_sfx_opcodes.tsv" --effect "$effect" --effect-every 37 --effect-voice 3
fi

# VICE logs every SID write with its cycle (cycle-exact 6510, VIC and CIA). Its WAV device never flushes when the cycle
# limit stops it, so the log is replayed through score64-emu's own SID instead.
vsid -default -silent -console -warp -limitcycles $cycles -sidenginemodel 256 \
  -sounddev dump -soundarg "$work/${tune}_vice.dmp" -tune 1 "$work/$tune.sid" >/dev/null 2>&1 || true
for chip in 6581 8580; do
  "$emu" replay "$work/${tune}_vice.dmp" --seconds $seconds --chip $chip --wav "$work/${tune}_vicetiming_$chip.wav"
done

uv run -q --no-project --with numpy --with scipy python "$here/compare.py" "$work" "$tune"

if [ -n "$effect" ] && [ -n "${KICKASS:-}" ]; then
  "$emu" render "$work/$tune.sid" --seconds $seconds --wav /dev/null --writes "$work/${tune}_sfx_helper_writes.tsv" \
    --effect "$effect" --effect-every 37 --effect-voice 3 >/dev/null
  java -jar "$KICKASS" "$here/sfx-driver.asm" -o "$work/sfx-driver.prg" -odir "$work" \
    ":tune=$work/$tune.sid" ":fx=$(cd "$(dirname "$effect")" && pwd)/$(basename "$effect")" >/dev/null
  # KERNAL boot takes about 3 s of emulated time before the driver starts.
  x64sc -default -silent -warp -limitcycles $(( (seconds + 15) * 985248 )) -sounddev dump \
    -soundarg "$work/${tune}_sfx_x64sc.dmp" -autostartprgmode 1 -autostart "$work/sfx-driver.prg" >/dev/null 2>&1 || true
  echo "#### With effects: helper vs x64sc + sfx-driver.prg"
  uv run -q --no-project --with numpy --with scipy python -c "
import sys; sys.path.insert(0, '$here')
import compare as c
c.compare_writes(c.helper_frames('$work/${tune}_sfx_helper_writes.tsv'), c.vice_bursts('$work/${tune}_sfx_x64sc.dmp'))"
fi
