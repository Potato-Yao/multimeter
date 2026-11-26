#!/usr/bin/env bash
set -Eeuo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRIPT_PATH="${SCRIPT_DIR}/$(basename -- "${BASH_SOURCE[0]}")"
readonly REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"

usage() {
    cat <<'EOF'
Usage: scripts/test-local.sh [OPTIONS]

Launch an already-built multimeter executable with administrator privileges,
capture its one-time WebSocket credentials, and open an authenticated interactive
websocat session.

Options:
  --executable PATH        multimeter executable path
  --websocat PATH          websocat executable or command name (default: websocat)
  --startup-timeout SEC    seconds to wait for launch information (default: 30)
  -h, --help               print this help

Without --executable, the script checks target/release and target/debug.
Build multimeter as your normal user before running this script. The script
requests elevation with sudo when it is not already running as root.
EOF
}

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

resolve_executable() {
    local candidate=$1
    local resolved

    if [[ ${candidate} == */* ]]; then
        [[ -f ${candidate} && -x ${candidate} ]] ||
            die "multimeter executable is missing or not executable: ${candidate}"
        resolved="$(cd -- "$(dirname -- "${candidate}")" && pwd)/$(basename -- "${candidate}")"
    else
        resolved="$(type -P -- "${candidate}" || true)"
        [[ -n ${resolved} && -x ${resolved} ]] ||
            die "multimeter executable was not found in PATH: ${candidate}"
    fi

    printf '%s\n' "${resolved}"
}

resolve_websocat() {
    local candidate=$1
    local resolved

    if [[ ${candidate} == */* ]]; then
        [[ -f ${candidate} && -x ${candidate} ]] ||
            die "websocat executable is missing or not executable: ${candidate}"
        resolved="$(cd -- "$(dirname -- "${candidate}")" && pwd)/$(basename -- "${candidate}")"
    else
        resolved="$(type -P -- "${candidate}" || true)"
        [[ -n ${resolved} && -x ${resolved} ]] ||
            die "websocat executable was not found in PATH: ${candidate}"
    fi

    printf '%s\n' "${resolved}"
}

default_executable() {
    local candidate
    local candidates=(
        "${REPO_ROOT}/target/release/multimeter"
        "${REPO_ROOT}/target/debug/multimeter"
        "${REPO_ROOT}/target/x86_64-unknown-linux-gnu/release/multimeter"
        "${REPO_ROOT}/target/x86_64-unknown-linux-musl/release/multimeter"
        "${REPO_ROOT}/target/aarch64-unknown-linux-gnu/release/multimeter"
    )

    for candidate in "${candidates[@]}"; do
        if [[ -f ${candidate} && -x ${candidate} ]]; then
            printf '%s\n' "${candidate}"
            return
        fi
    done

    die "no built multimeter executable found; pass --executable PATH"
}

executable_arg=
websocat_arg=websocat
startup_timeout=30

while (($# > 0)); do
    case $1 in
        --executable)
            (($# >= 2)) || die "--executable requires a path"
            executable_arg=$2
            shift 2
            ;;
        --websocat)
            (($# >= 2)) || die "--websocat requires a path or command name"
            websocat_arg=$2
            shift 2
            ;;
        --startup-timeout)
            (($# >= 2)) || die "--startup-timeout requires a positive integer"
            startup_timeout=$2
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            die "unknown argument: $1"
            ;;
    esac
done

[[ ${startup_timeout} =~ ^[0-9]+$ ]] &&
    ((startup_timeout > 0)) ||
    die "--startup-timeout must be a positive integer"

if [[ -n ${executable_arg} ]]; then
    executable_path=$(resolve_executable "${executable_arg}")
else
    executable_path=$(default_executable)
fi
websocat_path=$(resolve_websocat "${websocat_arg}")

if ((EUID != 0)); then
    command -v sudo >/dev/null 2>&1 ||
        die "sudo is required to launch multimeter with administrator privileges"
    exec sudo -- "${SCRIPT_PATH}" \
        --executable "${executable_path}" \
        --websocat "${websocat_path}" \
        --startup-timeout "${startup_timeout}"
fi

temp_dir=
multimeter_pid=

cleanup() {
    local exit_code=$?
    trap - EXIT INT TERM

    if [[ -n ${multimeter_pid} ]] && kill -0 "${multimeter_pid}" 2>/dev/null; then
        kill "${multimeter_pid}" 2>/dev/null || true
        wait "${multimeter_pid}" 2>/dev/null || true
    fi
    if [[ -n ${temp_dir} && -d ${temp_dir} ]]; then
        rm -rf -- "${temp_dir}"
    fi

    exit "${exit_code}"
}

trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

temp_dir=$(mktemp -d "${TMPDIR:-/tmp}/multimeter-local-test.XXXXXX")
chmod 700 "${temp_dir}"
log_file="${temp_dir}/multimeter.log"

printf 'Launching elevated multimeter: %s\n' "${executable_path}"
"${executable_path}" --no-browser >"${log_file}" 2>&1 &
multimeter_pid=$!

deadline=$((SECONDS + startup_timeout))
launch_url=
while ((SECONDS < deadline)); do
    if ! kill -0 "${multimeter_pid}" 2>/dev/null; then
        wait "${multimeter_pid}" 2>/dev/null || true
        printf 'multimeter exited before publishing launch information:\n' >&2
        sed -n '1,120p' "${log_file}" >&2
        exit 1
    fi

    launch_url=$(sed -n 's/^launch URL: //p' "${log_file}" | head -n 1 | tr -d '\r')
    if [[ -n ${launch_url} ]]; then
        break
    fi
    sleep 0.1
done

if [[ -z ${launch_url} ]]; then
    printf 'multimeter did not publish launch information within %s seconds:\n' \
        "${startup_timeout}" >&2
    sed -n '1,120p' "${log_file}" >&2
    exit 1
fi

fragment=${launch_url#*#multimeter=}
[[ ${fragment} != "${launch_url}" && -n ${fragment} ]] ||
    die "launch URL does not contain a multimeter bootstrap fragment"
[[ ${fragment} =~ ^[A-Za-z0-9_-]+$ ]] ||
    die "launch URL contains an invalid bootstrap fragment"

case $((${#fragment} % 4)) in
    0) padding= ;;
    2) padding='==' ;;
    3) padding='=' ;;
    *) die "launch URL contains an invalid bootstrap fragment length" ;;
esac

if ! bootstrap_json=$(
    printf '%s' "${fragment}${padding}" |
        tr '_-' '/+' |
        base64 -d 2>/dev/null
); then
    die "could not decode the multimeter bootstrap fragment"
fi

ws_url=$(printf '%s' "${bootstrap_json}" | sed -n 's/.*"ws":"\([^"]*\)".*/\1/p')
token=$(printf '%s' "${bootstrap_json}" | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')

[[ ${bootstrap_json} == *'"v":1'* ]] ||
    die "unsupported multimeter bootstrap protocol version"
[[ ${ws_url} == ws://* || ${ws_url} == wss://* ]] ||
    die "bootstrap data does not contain a valid WebSocket URL"
[[ ${token} =~ ^[A-Za-z0-9_-]+$ ]] ||
    die "bootstrap data does not contain a valid authentication token"

auth_payload=$(printf '{"type":"authenticate","token":"%s"}' "${token}")

printf '\nWebSocket URL: %s\n' "${ws_url}"
printf 'Authentication token extracted (%s characters).\n' "${#token}"
printf 'Starting websocat; the authenticate message is sent automatically.\n'
printf 'Type one JSON command per line. Exit websocat to stop multimeter.\n\n'

websocat_status=0
"${websocat_path}" -t "--preamble=${auth_payload}" "${ws_url}" || websocat_status=$?
exit "${websocat_status}"
