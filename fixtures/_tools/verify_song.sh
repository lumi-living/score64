#!/bin/sh
# Pack the GoatTracker fixture song and check it plays the same in score64-emu, VICE and sidplayfp (#18).
#
#   KICKASS=/path/to/KickAss.jar fixtures/_tools/verify_song.sh <workdir>
#
# Needs gt2reloc, ins2snd2 (GoatTracker 2.77), sidplayfp, vsid and x64sc (VICE 3.10), java and uv on PATH; see
# docs/TOOLING.md. Outputs (packed files, logs, WAVs) land in <workdir>, which should be git-ignored. Packed files
# contain the GoatTracker player and are not committed.
set -eu

work=$(mkdir -p "$1" && cd "$1" && pwd)
root=$(cd "$(dirname "$0")/../.." && pwd)
fix="$root/fixtures/gt2-original"
spike="$root/crates/score64-emu/spikes/packed-player"
seconds=31            # two loops of the 8-bar piece at 125 BPM
cycles=$(( (seconds + 1) * 985248 ))

python3 -I "$root/fixtures/_tools/make_song.py" "$fix" >/dev/null
for k in footstep gunshot clink; do ins2snd2 "$fix/sfx-$k.ins" "$fix/sfx-$k.bin" -b >/dev/null; done
# gt2reloc keeps file names in fixed-size buffers and crashes on long paths: pack inside the work directory.
cp "$fix/sixteen-steps.sng" "$work/"
(cd "$work" && gt2reloc sixteen-steps.sng sixteen_d1h1.sid -D1 -H1 -ZF8 >/dev/null &&
  gt2reloc sixteen-steps.sng sixteen_plain.sid -ZF8 >/dev/null)

cargo build --release -q -p score64-emu --manifest-path "$root/Cargo.toml"
emu="$root/target/release/score64-emu"
fx="--effect $fix/sfx-footstep.bin --effect $fix/sfx-gunshot.bin --effect $fix/sfx-clink.bin --effect-every 37 --effect-voice 3"

compare() {  # helper writes, VICE dump
  uv run -q --no-project --with numpy --with scipy python -c "
import sys; sys.path.insert(0, '$spike')
import compare as c
c.compare_writes(c.helper_frames('$1'), c.vice_bursts('$2'))" | grep -E "identical|all writes|jitter"
}

for build in d1h1 plain; do
  for st in 1 2; do
    t="$work/sixteen_${build}_st$st"
    echo "## $build, subtune $st"
    "$emu" render "$work/sixteen_$build.sid" --subtune $st --seconds $seconds --wav "$t.helper_6581.wav" \
      --writes "$t.helper_writes.tsv" --opcodes "$t.opcodes.tsv"
    "$emu" render "$work/sixteen_$build.sid" --subtune $st --seconds $seconds --chip 8580 --wav "$t.helper_8580.wav" >/dev/null
    sidplayfp -t$seconds -mof --delay=0 -o$st -w"$t.sidplayfp_6581.wav" "$work/sixteen_$build.sid" >/dev/null 2>&1
    sidplayfp -t$seconds -mnf --delay=0 -o$st -w"$t.sidplayfp_8580.wav" "$work/sixteen_$build.sid" >/dev/null 2>&1
    vsid -default -silent -console -warp -limitcycles $cycles -sounddev dump -soundarg "$t.vice.dmp" \
      -tune $st "$work/sixteen_$build.sid" >/dev/null 2>&1 || true
    compare "$t.helper_writes.tsv" "$t.vice.dmp"
  done
done

if [ -n "${KICKASS:-}" ]; then
  for st in 1 2; do
    t="$work/sixteen_d1h1_st${st}_sfx"
    echo "## d1h1, subtune $st, footstep / gunshot / clink on voice 3 every 37 frames"
    # shellcheck disable=SC2086
    "$emu" render "$work/sixteen_d1h1.sid" --subtune $st --seconds $seconds $fx --wav "$t.helper_6581.wav" \
      --writes "$t.helper_writes.tsv" --opcodes "$t.opcodes.tsv"
    java -jar "$KICKASS" "$spike/sfx-driver.asm" -o "$t.prg" -odir "$work" ":tune=$work/sixteen_d1h1.sid" \
      ":fx1=$fix/sfx-footstep.bin" ":fx2=$fix/sfx-gunshot.bin" ":fx3=$fix/sfx-clink.bin" ":subtune=$((st - 1))" >/dev/null
    x64sc -default -silent -warp -limitcycles $(( cycles + 15 * 985248 )) -sounddev dump -soundarg "$t.x64sc.dmp" \
      -autostartprgmode 1 -autostart "$t.prg" >/dev/null 2>&1 || true
    compare "$t.helper_writes.tsv" "$t.x64sc.dmp"
  done
fi
