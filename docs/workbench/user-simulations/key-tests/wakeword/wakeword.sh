#!/bin/sh
# wakeword.sh - start/stop the "hey phone" wake-word listener (KEY TEST 1).
#
#   wakeword.sh start        engine + warm services + listener on
#   wakeword.sh stop         listener off, warm services off, engine removed
#   wakeword.sh status       what's running + last log lines
#   wakeword.sh engine-start / engine-stop   just the Moonshine container
#   wakeword.sh warm-start / warm-stop       just the CV0 warm services
#
# CV0: two persistent speech services (voiced-warm-stt = Moonshine
# `small`, voiced-warm-tts = ptts/Phonon) run as systemd user units
# alongside the listener, so a fired turn reuses loaded models
# instead of paying a throwaway container start per stage. Their
# lifecycle is tied to the listener's: start brings them up first,
# stop takes them down. Fired turns prefer ~/agentic/voiced-cv0.conf
# (warm adapters + instant ack); the cold configs are untouched.
#   wakeword.sh test-file X.wav              file-mode detector test
#
# CV2: a third warm service, voiced-smartturn (Pipecat Smart Turn v3
# endpointing judge), is managed with the warm pair but BEST-EFFORT:
# if it fails to start, start still succeeds and both capture paths
# fall back to the CV0 endpointing (journalled / logged). Provision
# it once with smartturn-setup.sh.
#
# The listener runs as a systemd user unit (wakeword-listener), NOT as a
# nohup child of the invoking shell: a plain nohup listener is reaped when
# the SSH session that started it closes, while the unit survives session
# teardown (same mechanism agentd already relies on). The unit's MainPID
# is recorded in listener.pid, and stop/status act on BOTH the unit and
# the pidfile, so a legacy nohup listener is still managed correctly.
#
# Everything lives under ~/agentic/wakeword (additive). Nothing here
# touches the deployed Track A files; the only pre-existing file it may
# create is ~/agentic/voiced-tonight.conf, with exactly the sed the
# KEY-TEST-1 run sheet Step 2 prescribes, and only if it is absent.
set -u
WW_HOME=${WW_HOME:-$HOME/agentic/wakeword}
ENGINE_NAME=agentic-wakeword
IMAGE=localhost/voice-snapshot:20261024
ARCH=${WW_ARCH:-tiny}
PIDFILE=$WW_HOME/listener.pid
LOG=$WW_HOME/listener.log
UNIT=wakeword-listener
WARM_STT_UNIT=voiced-warm-stt
WARM_TTS_UNIT=voiced-warm-tts
WARM_STT_DIR=$HOME/agentic/warm-stt
WARM_TTS_DIR=$HOME/agentic/warm-tts
SMARTTURN_UNIT=voiced-smartturn
SMARTTURN_DIR=$HOME/agentic/smartturn
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
mkdir -p "$WW_HOME/inbox" "$WW_HOME/outbox" "$WW_HOME/heard"

engine_running() {
    sudo -n podman ps --format '{{.Names}}' 2>/dev/null | grep -qx "$ENGINE_NAME"
}

engine_start() {
    if engine_running; then
        echo "engine already running"
        return 0
    fi
    rm -f "$WW_HOME/STOP" "$WW_HOME/outbox/READY"
    sudo -n podman rm -f "$ENGINE_NAME" >/dev/null 2>&1 || true
    sudo -n podman run -d --name "$ENGINE_NAME" --network none --userns=keep-id \
        -e MOONSHINE_VOICE_CACHE=/opt/models/moonshine -e STT_ARCH="$ARCH" \
        -e WW_DIR=/ww \
        -v /opt/models:/opt/models:ro \
        -v "$WW_HOME:/ww" \
        -v $HOME/.local/bin/voice-listen-app/brainkit/scripts:/bk:ro \
        "$IMAGE" /opt/stt/venv/bin/python /ww/ww_engine.py >/dev/null || {
        echo "ERROR: engine container failed to start" >&2; return 1; }
    i=0
    while [ $i -lt 120 ]; do
        [ -f "$WW_HOME/outbox/READY" ] && {
            echo "engine ready: $(cat "$WW_HOME/outbox/READY")"; return 0; }
        engine_running || { echo "ERROR: engine died during load:" >&2;
            sudo -n podman logs --tail 5 "$ENGINE_NAME" >&2; return 1; }
        sleep 1; i=$((i + 1))
    done
    echo "ERROR: engine not READY after 120s" >&2
    return 1
}

engine_stop() {
    touch "$WW_HOME/STOP"
    sleep 1
    sudo -n podman stop -t 5 "$ENGINE_NAME" >/dev/null 2>&1 || true
    sudo -n podman rm -f "$ENGINE_NAME" >/dev/null 2>&1 || true
    rm -f "$WW_HOME/STOP" "$WW_HOME/outbox/READY"
    echo "engine stopped"
}

warm_running() {
    systemctl --user is-active --quiet "$1" 2>/dev/null
}

warm_start() {
    mkdir -p "$WARM_STT_DIR/inbox" "$WARM_STT_DIR/outbox" \
        "$WARM_TTS_DIR/inbox" "$WARM_TTS_DIR/outbox"
    rm -f "$WARM_STT_DIR/STOP" "$WARM_TTS_DIR/STOP" \
        "$WARM_STT_DIR/outbox/READY" "$WARM_TTS_DIR/outbox/READY"
    systemctl --user start "$WARM_STT_UNIT" || {
        echo "ERROR: unit $WARM_STT_UNIT failed to start" >&2; return 1; }
    systemctl --user start "$WARM_TTS_UNIT" || {
        echo "ERROR: unit $WARM_TTS_UNIT failed to start" >&2; return 1; }
    i=0
    while [ $i -lt 120 ]; do
        if [ -f "$WARM_STT_DIR/outbox/READY" ] && [ -f "$WARM_TTS_DIR/outbox/READY" ]; then
            echo "warm stt ready: $(cat "$WARM_STT_DIR/outbox/READY")"
            echo "warm tts ready: $(cat "$WARM_TTS_DIR/outbox/READY")"
            return 0
        fi
        warm_running "$WARM_STT_UNIT" || {
            echo "ERROR: unit $WARM_STT_UNIT died during load:" >&2
            systemctl --user status "$WARM_STT_UNIT" --no-pager 2>&1 | tail -5 >&2
            return 1; }
        warm_running "$WARM_TTS_UNIT" || {
            echo "ERROR: unit $WARM_TTS_UNIT died during load:" >&2
            systemctl --user status "$WARM_TTS_UNIT" --no-pager 2>&1 | tail -5 >&2
            return 1; }
        sleep 1; i=$((i + 1))
    done
    echo "ERROR: warm services not READY after 120s" >&2
    return 1
}

# CV2: the Smart Turn judge is best-effort BY DESIGN - its absence is
# a supported state (the CV0 fallback), so a start failure warns and
# returns success instead of failing the listener start.
smartturn_start() {
    mkdir -p "$SMARTTURN_DIR/inbox" "$SMARTTURN_DIR/outbox"
    rm -f "$SMARTTURN_DIR/STOP" "$SMARTTURN_DIR/READY" "$SMARTTURN_DIR/FAILED"
    systemctl --user start "$SMARTTURN_UNIT" 2>/dev/null || {
        echo "WARNING: unit $SMARTTURN_UNIT failed to start; Smart Turn OFF, CV0 endpointing fallback in effect" >&2
        return 0; }
    i=0
    while [ $i -lt 30 ]; do
        if [ -f "$SMARTTURN_DIR/READY" ]; then
            echo "smartturn ready: model sha256 $(cut -c1-12 "$SMARTTURN_DIR/READY")..."
            return 0
        fi
        if [ -f "$SMARTTURN_DIR/FAILED" ] || ! warm_running "$SMARTTURN_UNIT"; then
            break
        fi
        sleep 1; i=$((i + 1))
    done
    echo "WARNING: smartturn not READY after ${i}s; Smart Turn OFF, CV0 endpointing fallback in effect" >&2
    return 0
}

smartturn_stop() {
    systemctl --user stop "$SMARTTURN_UNIT" 2>/dev/null || true
    systemctl --user reset-failed "$SMARTTURN_UNIT" >/dev/null 2>&1 || true
    rm -f "$SMARTTURN_DIR/READY" "$SMARTTURN_DIR/FAILED"
}

warm_stop() {
    systemctl --user stop "$WARM_STT_UNIT" "$WARM_TTS_UNIT" 2>/dev/null || true
    systemctl --user reset-failed "$WARM_STT_UNIT" "$WARM_TTS_UNIT" >/dev/null 2>&1 || true
    rm -f "$WARM_STT_DIR/outbox/READY" "$WARM_TTS_DIR/outbox/READY"
    smartturn_stop
    echo "warm services stopped"
}

unit_active() {
    systemctl --user is-active --quiet "$UNIT" 2>/dev/null
}

unit_pid() {
    systemctl --user show "$UNIT" -p MainPID --value 2>/dev/null
}

pidfile_alive() {
    [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null
}

listener_running() {
    unit_active || pidfile_alive
}

listener_pid() {
    if unit_active; then
        unit_pid
    elif [ -f "$PIDFILE" ]; then
        cat "$PIDFILE"
    fi
}

cmd_start() {
    if listener_running; then
        echo "listener already running (pid $(listener_pid))"; return 0
    fi
    engine_start || return 1
    warm_start || return 1
    smartturn_start
    # tonight's audible voiced conf (run sheet Step 2), only if absent
    if [ ! -f "$HOME/agentic/voiced-tonight.conf" ]; then
        sed "s/^play = none/play = paplay/; s/^quiet = true/quiet = false/" \
            "$HOME/agentic/voiced.conf" > "$HOME/agentic/voiced-tonight.conf"
        echo "created ~/agentic/voiced-tonight.conf (run sheet Step 2)"
    fi
    # Launch as a systemd user unit so the listener survives the session
    # that started it. Stdout/stderr append to the listener log, exactly
    # where the old nohup redirect put them. Listener env knobs are
    # passed through when the caller set them.
    SETENV=""
    [ -n "${WW_ONCE:-}" ] && SETENV="$SETENV --setenv=WW_ONCE=$WW_ONCE"
    [ -n "${WW_RMS_MIN:-}" ] && SETENV="$SETENV --setenv=WW_RMS_MIN=$WW_RMS_MIN"
    [ -n "${WW_TIMEOUT:-}" ] && SETENV="$SETENV --setenv=WW_TIMEOUT=$WW_TIMEOUT"
    [ -n "${WW_FOLLOWUP_S:-}" ] && SETENV="$SETENV --setenv=WW_FOLLOWUP_S=$WW_FOLLOWUP_S"
    [ -n "${WW_AGENTD:-}" ] && SETENV="$SETENV --setenv=WW_AGENTD=$WW_AGENTD"
    # shellcheck disable=SC2086
    systemd-run --user --unit="$UNIT" --collect $SETENV \
        /bin/sh -c "exec python3 $WW_HOME/wakeword.py >> $LOG 2>&1" || {
        echo "ERROR: systemd-run failed to start unit $UNIT" >&2; return 1; }
    i=0; pid=""
    while [ $i -lt 15 ]; do
        pid=$(unit_pid)
        [ -n "$pid" ] && [ "$pid" != "0" ] && unit_active && break
        pid=""; sleep 1; i=$((i + 1))
    done
    [ -n "$pid" ] || {
        echo "ERROR: unit $UNIT did not come up; last log lines:" >&2
        tail -5 "$LOG" >&2; return 1; }
    echo "$pid" > "$PIDFILE"
    echo "listening (pid $pid, unit $UNIT, log: $LOG)"
    echo "NOTE: agentd must be up for the fired turn to reach the agent"
    echo "      (KEY-TEST-1 run sheet Step 1)."
}

cmd_stop() {
    stopped=0
    if unit_active; then
        systemctl --user stop "$UNIT" 2>/dev/null || true
        stopped=1
    fi
    systemctl --user reset-failed "$UNIT" >/dev/null 2>&1 || true
    if [ -f "$PIDFILE" ]; then
        if kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
            kill "$(cat "$PIDFILE")" 2>/dev/null || true
            sleep 1
            kill -9 "$(cat "$PIDFILE")" 2>/dev/null || true
            stopped=1
        fi
        rm -f "$PIDFILE"
    fi
    if [ "$stopped" = 1 ]; then
        echo "listener stopped"
    else
        echo "listener not running"
    fi
    warm_stop
    engine_stop
}

cmd_status() {
    if unit_active; then
        echo "listener: RUNNING pid $(unit_pid) (systemd unit $UNIT)"
    elif pidfile_alive; then
        echo "listener: RUNNING pid $(cat "$PIDFILE") (pidfile only, no unit)"
    else
        echo "listener: not running"
    fi
    if engine_running; then
        echo "engine:   RUNNING (container $ENGINE_NAME, arch $ARCH)"
    else
        echo "engine:   not running"
    fi
    if warm_running "$WARM_STT_UNIT"; then
        echo "warm stt: RUNNING (unit $WARM_STT_UNIT, container agentic-warm-stt, arch small)"
    else
        echo "warm stt: not running"
    fi
    if warm_running "$WARM_TTS_UNIT"; then
        echo "warm tts: RUNNING (unit $WARM_TTS_UNIT, container agentic-warm-tts)"
    else
        echo "warm tts: not running"
    fi
    if warm_running "$SMARTTURN_UNIT"; then
        echo "smartturn: RUNNING (unit $SMARTTURN_UNIT, container agentic-smartturn)"
    else
        echo "smartturn: not running (CV0 endpointing fallback in effect)"
    fi
    [ -f "$LOG" ] && { echo "--- last log lines ---"; tail -5 "$LOG"; }
}

case "${1:-}" in
    start) cmd_start ;;
    stop) cmd_stop ;;
    status) cmd_status ;;
    engine-start) engine_start ;;
    engine-stop) engine_stop ;;
    warm-start) warm_start ;;
    warm-stop) warm_stop ;;
    test-file)
        [ -n "${2:-}" ] || { echo "usage: wakeword.sh test-file X.wav" >&2; exit 2; }
        STARTED=0
        engine_running || { engine_start || exit 1; STARTED=1; }
        python3 "$WW_HOME/wakeword.py" --file "$2"
        rc=$?
        [ "$STARTED" = 1 ] && engine_stop
        exit $rc ;;
    *) echo "usage: wakeword.sh start|stop|status|engine-start|engine-stop|warm-start|warm-stop|test-file X.wav" >&2; exit 2 ;;
esac
