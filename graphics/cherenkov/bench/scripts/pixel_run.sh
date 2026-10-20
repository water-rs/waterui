#!/system/bin/sh
# Own one benchmark process and its temporary device configuration.
# Arguments are repeated --node PATH VALUE, --property KEY VALUE,
# --clock NAME POLICY_DIR KHZ (a cpufreq policy or the GPU:
# scaling_min_freq/scaling_max_freq), --devfreq NAME DEVFREQ_DIR KHZ
# (min_freq/max_freq), --residency NAME FILE (a time-in-state table read
# before and after the workload), --sensor NAME PATH MAX_MILLICELSIUS,
# followed by -- COMMAND ARGUMENTS.
# The caller supplies a fresh --out directory and a positive --seconds bound.
set -u
umask 022
out=
seconds=
controls=0
clocks=0
residencies=0
worker=
# "NAME:PATH:LIMIT" per sensor, and every reading taken, both held in the
# shell so a check opens nothing but the sensors themselves.
sensors=
readings=
checks=0

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
    if [ -n "$out" ] && [ -n "$sensors" ]; then
        printf '%s' "$readings" > "$out/temperature-samples" || result=1
    fi
    index=$controls
    while [ "$index" -gt 0 ]; do
        index=$((index - 1))
        entry="$out/control-$index"
        read -r kind < "$entry.kind"
        read -r key < "$entry.key"
        read -r value < "$entry.original"
        if [ "$kind" = clock ]; then
            read -r low < "$entry.min-node"
            read -r high_node < "$entry.max-node"
            read -r high < "$entry.original-max"
            printf '%s\n' "$high" > "$key/$high_node" || result=1
            printf '%s\n' "$value" > "$key/$low" || result=1
            printf '%s\n' "$high" > "$key/$high_node" || result=1
            cat "$key/$low" > "$entry.restored" || result=1
            cat "$key/$high_node" > "$entry.restored-max" || result=1
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

# A clock held at one frequency: its name, directory and the two nodes
# that bound it.
register_clock() {
    entry="$out/control-$controls"
    printf 'clock\n' > "$entry.kind"
    printf '%s\n' "$2" > "$entry.key"
    printf '%s\n' "$3" > "$entry.requested"
    printf '%s\n' "$1" > "$entry.name"
    printf '%s\n' "$4" > "$entry.min-node"
    printf '%s\n' "$5" > "$entry.max-node"
    cat "$2/$4" > "$entry.original" || exit 1
    cat "$2/$5" > "$entry.original-max" || exit 1
    printf '%s\n' "$controls" > "$out/clock-$clocks"
    controls=$((controls + 1))
    printf '%s\n' "$controls" > "$out/registered-controls"
    clocks=$((clocks + 1))
}

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
            register_clock "$2" "$3" "$4" scaling_min_freq scaling_max_freq
            shift 4
            ;;
        --devfreq)
            [ -n "$out" ] || exit 2
            register_clock "$2" "$3" "$4" min_freq max_freq
            shift 4
            ;;
        --residency)
            [ -n "$out" ] || exit 2
            printf '%s\n' "$2" > "$out/residency-$residencies.name"
            printf '%s\n' "$3" > "$out/residency-$residencies.path"
            residencies=$((residencies + 1))
            shift 3
            ;;
        --sensor)
            [ -n "$out" ] || exit 2
            case "$2:$3:$4" in
                *[[:space:]]*|*:*:*:*) echo "invalid sensor: $2 $3 $4" >&2; exit 2 ;;
            esac
            sensors="$sensors $2:$3:$4"
            shift 4
            ;;
        --)
            shift
            break
            ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done
[ -n "$out" ] && [ -n "$seconds" ] && [ "$seconds" -gt 0 ] && [ -n "$sensors" ] && [ "$#" -gt 0 ] || exit 2
# One sensor per line; the list splits on the spaces between entries.
# shellcheck disable=SC2086
printf '%s\n' $sensors > "$out/sensors"

# Reads every sensor once with builtins only, so a check forks nothing.
check_temperatures() {
    line=
    for sensor in $sensors; do
        name=${sensor%%:*}
        rest=${sensor#*:}
        path=${rest%:*}
        limit=${rest##*:}
        read -r value < "$path" || return 1
        case "$value" in
            ''|*[!0-9]*) echo "invalid temperature reading: $path" >&2; return 1 ;;
        esac
        line="$line $name=$value"
        if [ "$value" -ge "$limit" ]; then
            readings="$readings$checks$line
"
            echo "temperature limit: $name $value >= $limit mC" >&2
            return 1
        fi
    done
    readings="$readings$checks$line
"
    checks=$((checks + 1))
}
check_temperatures || exit 125
index=0
while [ "$index" -lt "$controls" ]; do
    entry="$out/control-$index"
    read -r kind < "$entry.kind"
    read -r key < "$entry.key"
    read -r value < "$entry.requested"
    if [ "$kind" = clock ]; then
        read -r low < "$entry.min-node"
        read -r high < "$entry.max-node"
        printf '%s\n' "$value" > "$key/$high" || exit 1
        printf '%s\n' "$value" > "$key/$low" || exit 1
        printf '%s\n' "$value" > "$key/$high" || exit 1
        cat "$key/$low" > "$entry.applied" || exit 1
        cat "$key/$high" > "$entry.applied-max" || exit 1
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
        read -r low < "$entry.min-node"
        read -r high < "$entry.max-node"
        cat "$key/$low" "$key/$high" > "$out/$name-$moment-range" || return 1
        clock=$((clock + 1))
    done
    index=0
    while [ "$index" -lt "$residencies" ]; do
        read -r name < "$out/residency-$index.name"
        read -r path < "$out/residency-$index.path"
        cat "$path" > "$out/$name-$moment-residency" || return 1
        index=$((index + 1))
    done
}
record_clocks before || exit 1

# The temperature checks wait on a FIFO nobody writes, a read with a
# timeout: a wait that forks nothing. The open descriptor keeps the FIFO
# alive once its name is gone.
mkfifo "$out/tick" || exit 1
exec 3<>"$out/tick"
rm "$out/tick" || exit 1

# Every process's cumulative CPU time and every CPU's tick counters, taken
# outside the workload's lifetime so they cost its frames nothing. A process
# that exits while `cat` walks the glob makes it fail; the rest is kept.
snapshot_processes() {
    cat /proc/uptime /proc/stat > "$out/system-$1" || return 1
    cat /proc/[0-9]*/stat > "$out/processes-$1" 2>/dev/null
    [ -s "$out/processes-$1" ]
}
snapshot_processes before || exit 1

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
    # mksh, Android's sh, reads with a fractional timeout.
    # shellcheck disable=SC3045
    # The workload's exit interrupts the read early, which ends the wait.
    read -t 0.25 -r _ <&3 2>/dev/null
done
wait "$worker"
result=$?
worker=
snapshot_processes after || exit 1
record_clocks after || exit 1
exit "$result"
