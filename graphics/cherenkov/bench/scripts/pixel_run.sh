#!/system/bin/sh
# Own one benchmark process and its temporary device configuration.
# Arguments are repeated --node PATH VALUE, --property KEY VALUE,
# --sensor NAME PATH MAX_MILLICELSIUS, followed by -- COMMAND ARGUMENTS.
# The caller supplies a fresh --out directory and a positive --seconds bound.
set -u
umask 022
out=
seconds=
controls=0
sensors=0
clocks=0
worker=

# Called by the EXIT trap, including signal and error exits.
# shellcheck disable=SC2329
restore() {
    result=$?
    trap - EXIT HUP INT TERM
    if [ -n "$worker" ]; then
        kill -TERM -- "-$worker" 2>/dev/null || :
        kill -KILL -- "-$worker" 2>/dev/null || :
        wait "$worker" 2>/dev/null || :
    fi
    index=$controls
    while [ "$index" -gt 0 ]; do
        index=$((index - 1))
        entry="$out/control-$index"
        read -r kind < "$entry.kind"
        read -r key < "$entry.key"
        read -r value < "$entry.original"
        if [ "$kind" = clock ]; then
            read -r high < "$entry.original-max"
            printf '%s\n' "$high" > "$key/scaling_max_freq" || result=1
            printf '%s\n' "$value" > "$key/scaling_min_freq" || result=1
            printf '%s\n' "$high" > "$key/scaling_max_freq" || result=1
            cat "$key/scaling_min_freq" > "$entry.restored" || result=1
            cat "$key/scaling_max_freq" > "$entry.restored-max" || result=1
            cmp -s "$entry.original-max" "$entry.restored-max" || result=1
        elif [ "$kind" = property ]; then
            setprop "$key" "$value" || result=1
            getprop "$key" > "$entry.restored" || result=1
        else
            printf '%s\n' "$value" > "$key" || result=1
            cat "$key" > "$entry.restored" || result=1
        fi
        if ! cmp -s "$entry.original" "$entry.restored"; then
            echo "restore mismatch: $key" >&2
            result=1
        fi
    done
    if [ -n "$out" ]; then
        printf '%s\n' "$result" > "$out/exit-code"
    fi
    exit "$result"
}
trap restore EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

while [ "$#" -gt 0 ]; do
    case "$1" in
        --out)
            destination=$2
            mkdir "$destination" || exit 1
            out=$destination
            printf '0\n' > "$out/registered-controls"
            printf '%s\n' "$$" > "$out/supervisor-pid"
            shift 2
            ;;
        --seconds)
            seconds=$2
            shift 2
            ;;
        --node|--property)
            [ -n "$out" ] || exit 2
            kind=${1#--}
            key=$2
            value=$3
            entry="$out/control-$controls"
            printf '%s\n' "$kind" > "$entry.kind"
            printf '%s\n' "$key" > "$entry.key"
            printf '%s\n' "$value" > "$entry.requested"
            if [ "$kind" = property ]; then
                getprop "$key" > "$entry.original" || exit 1
            else
                cat "$key" > "$entry.original" || exit 1
            fi
            controls=$((controls + 1))
            printf '%s\n' "$controls" > "$out/registered-controls"
            shift 3
            ;;
        --clock)
            [ -n "$out" ] || exit 2
            entry="$out/control-$controls"
            printf 'clock\n' > "$entry.kind"
            printf '%s\n' "$3" > "$entry.key"
            printf '%s\n' "$4" > "$entry.requested"
            printf '%s\n' "$2" > "$entry.name"
            cat "$3/scaling_min_freq" > "$entry.original" || exit 1
            cat "$3/scaling_max_freq" > "$entry.original-max" || exit 1
            printf '%s\n' "$controls" > "$out/clock-$clocks"
            controls=$((controls + 1))
            printf '%s\n' "$controls" > "$out/registered-controls"
            clocks=$((clocks + 1))
            shift 4
            ;;
        --sensor)
            [ -n "$out" ] || exit 2
            entry="$out/sensor-$sensors"
            printf '%s\n' "$2" > "$entry.name"
            printf '%s\n' "$3" > "$entry.path"
            printf '%s\n' "$4" > "$entry.limit"
            sensors=$((sensors + 1))
            shift 4
            ;;
        --)
            shift
            break
            ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done
[ -n "$out" ] && [ -n "$seconds" ] && [ "$seconds" -gt 0 ] && [ "$sensors" -gt 0 ] && [ "$#" -gt 0 ] || exit 2

check_temperatures() {
    index=0
    while [ "$index" -lt "$sensors" ]; do
        entry="$out/sensor-$index"
        read -r path < "$entry.path"
        read -r limit < "$entry.limit"
        read -r value < "$path" || return 1
        case "$value:$limit" in
            *[!0-9:]*|:*|*:) echo "invalid temperature reading: $path" >&2; return 1 ;;
        esac
        printf '%s\n' "$value" >> "$entry.samples"
        if [ "$value" -ge "$limit" ]; then
            echo "temperature limit: $(cat "$entry.name") $value >= $limit mC" >&2
            return 1
        fi
        index=$((index + 1))
    done
}
check_temperatures || exit 125
index=0
while [ "$index" -lt "$controls" ]; do
    entry="$out/control-$index"
    read -r kind < "$entry.kind"
    read -r key < "$entry.key"
    read -r value < "$entry.requested"
    if [ "$kind" = clock ]; then
        printf '%s\n' "$value" > "$key/scaling_max_freq" || exit 1
        printf '%s\n' "$value" > "$key/scaling_min_freq" || exit 1
        printf '%s\n' "$value" > "$key/scaling_max_freq" || exit 1
        cat "$key/scaling_min_freq" > "$entry.applied" || exit 1
        cat "$key/scaling_max_freq" > "$entry.applied-max" || exit 1
        cmp -s "$entry.requested" "$entry.applied-max" || exit 1
    elif [ "$kind" = property ]; then
        setprop "$key" "$value" || exit 1
        getprop "$key" > "$entry.applied" || exit 1
    else
        printf '%s\n' "$value" > "$key" || exit 1
        cat "$key" > "$entry.applied" || exit 1
    fi
    cmp -s "$entry.requested" "$entry.applied" || exit 1
    index=$((index + 1))
done

record_clocks() {
    moment=$1
    clock=0
    while [ "$clock" -lt "$clocks" ]; do
        read -r index < "$out/clock-$clock"
        entry="$out/control-$index"
        read -r key < "$entry.key"
        read -r name < "$entry.name"
        cat "$key/scaling_min_freq" "$key/scaling_max_freq" > "$out/$name-$moment-range" || return 1
        if [ "$name" != gpu ]; then
            cat "$key/stats/time_in_state" > "$out/$name-$moment-residency" || return 1
        fi
        clock=$((clock + 1))
    done
}
record_clocks before || exit 1

# A separate process group allows cleanup to stop exactly this workload tree.
setsid "$@" &
worker=$!
printf '%s\n' "$worker" > "$out/workload-pid"
started=$SECONDS
while kill -0 "$worker" 2>/dev/null; do
    check_temperatures || exit 125
    now=$SECONDS
    if [ "$((now - started))" -ge "$seconds" ]; then
        echo "benchmark deadline: $seconds seconds" >&2
        exit 124
    fi
    sleep 0.25
done
wait "$worker"
result=$?
worker=
record_clocks after || exit 1
exit "$result"
