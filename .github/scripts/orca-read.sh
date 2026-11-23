#!/usr/bin/env bash
# Lets Orca read a program and writes down what it said.
#
#   .github/scripts/orca-read.sh <program> <keys-file> <out.txt>
#
# Everything is private to the run: an Xvfb display, a D-Bus session bus with its own AT-SPI
# bus, a home and a runtime directory under a temporary directory, and a speech-dispatcher whose
# sound goes to an ALSA device that plays nothing. Nothing is spoken on the speakers of whoever
# runs this and the desktop is left alone. So nobody listens either: what is written to <out.txt>
# is Orca's own record of what it sent to the speech synthesizer ("SPEECH OUTPUT" in its debug
# log), each line with its time and with the key that was sent before it.
#
# <keys-file> has one line per step: a key for `xdotool key` (Tab, shift+Tab, Return, ...),
# `type some text`, or `sleep <seconds>`. Lines that begin with # are skipped.
#
# Needs: orca, Xvfb, openbox, xdotool, dbus-run-session, at-spi2-core, speech-dispatcher with
# espeak-ng. Not run by CI.
set -euo pipefail

program="$(realpath "$1")"
keys="$(realpath "$2")"
out="$(realpath -m "$3")"
pause="${ORCA_READ_PAUSE:-1.8}"

work="$(mktemp -d /tmp/orca-read.XXXXXX)"
mkdir -p "$work/home/.config/speech-dispatcher" "$work/run"
chmod 700 "$work/run"

# Sound that goes nowhere, for anything that asks ALSA or speech-dispatcher
cat > "$work/home/.asoundrc" <<'ASOUND'
pcm.!default { type null }
ctl.!default { type null }
ASOUND
cat > "$work/home/.config/speech-dispatcher/speechd.conf" <<'SPEECHD'
LogLevel 1
AudioOutputMethod "alsa"
AudioALSADevice "null"
AddModule "espeak-ng" "sd_espeak-ng" "espeak-ng.conf"
DefaultModule espeak-ng
SPEECHD

display=""
for n in $(seq 90 110); do
    if [ ! -e "/tmp/.X11-unix/X$n" ]; then display=":$n"; break; fi
done
[ -n "$display" ] || { echo "no free display"; exit 1; }

Xvfb "$display" -screen 0 1280x1000x24 -nolisten tcp >/dev/null 2>&1 &
xvfb=$!
trap 'kill $xvfb 2>/dev/null || true; rm -rf "$work"' EXIT
sleep 1

env -i PATH="$PATH" HOME="$work/home" XDG_RUNTIME_DIR="$work/run" DISPLAY="$display" \
    LANG=en_US.UTF-8 ICED_BACKEND=tiny-skia GTK_MODULES= NO_AT_BRIDGE=0 \
    PROGRAM="$program" KEYS="$keys" WORK="$work" PAUSE="$pause" \
    dbus-run-session -- bash -c '
        set -u
        exec 2>>"$WORK/session.log"
        openbox >/dev/null 2>&1 &
        /usr/libexec/at-spi-bus-launcher --launch-immediately >/dev/null 2>&1 &
        sleep 1
        # A screen reader listens: toolkits build their trees
        busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled b true
        busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status ScreenReaderEnabled b true

        orca --debug-file="$WORK/orca.log" >/dev/null 2>&1 &
        orca=$!
        sleep 6

        "$PROGRAM" >"$WORK/program.log" 2>&1 &
        program=$!
        sleep 4

        : > "$WORK/keys.log"
        while IFS= read -r line; do
            case "$line" in
                ""|\#*) continue ;;
                sleep\ *) sleep "${line#sleep }"; continue ;;
                type\ *)
                    echo "$(date +%H:%M:%S.%N | cut -c1-15) type ${line#type }" >> "$WORK/keys.log"
                    xdotool type --delay 150 "${line#type }" ;;
                *)
                    echo "$(date +%H:%M:%S.%N | cut -c1-15) key $line" >> "$WORK/keys.log"
                    xdotool key "$line" ;;
            esac
            sleep "$PAUSE"
        done < "$KEYS"

        sleep 2
        kill $program 2>/dev/null
        kill $orca 2>/dev/null
        sleep 1
        pkill -u "$(id -u)" -f "speech-dispatcher.*$WORK" 2>/dev/null
        true
    '

# What Orca sent to the synthesizer, and the keys, by time
python3 - "$work/orca.log" "$work/keys.log" > "$out" <<'PY'
import re, sys
said = []
for line in open(sys.argv[1], errors="replace"):
    m = re.match(r"(\d\d:\d\d:\d\d\.\d+) - SPEECH OUTPUT: '(.*?)'(?: \{.*)?$", line.strip())
    if m:
        text = re.sub(r"\s+", " ", m.group(2)).strip()
        if text:
            said.append((m.group(1)[:15], "said  " + text))
keys = []
for line in open(sys.argv[2]):
    time, rest = line.rstrip("\n").split(" ", 1)
    keys.append((time, "      [" + rest + "]"))
for time, text in sorted(said + keys):
    print(time, text)
PY
if grep -q "panicked at" "$work/program.log"; then
    echo "THE PROGRAM PANICKED" >> "$out"
    cat "$work/program.log" >> "$out"
fi
cp "$work/orca.log" "${out%.txt}.debug.log" 2>/dev/null || true
echo "wrote $out ($(grep -c 'said ' "$out") things said)"
