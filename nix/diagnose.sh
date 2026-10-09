#!/usr/bin/env bash
# mizu-diagnose FILE.pdf
#
# Opens mizu several times with different graphics settings, asks which of
# them showed the page, and prints a report that can be pasted into an issue.
# Run it from a terminal inside the graphical session that has the problem.
set -u

file="${1:-}"
if [ -z "$file" ] || [ ! -r "$file" ]; then
    echo "usage: mizu-diagnose FILE.pdf" >&2
    exit 2
fi
secs="${MIZU_DIAGNOSE_SECONDS:-6}"
out="$(mktemp -d)"
answers="${MIZU_DIAGNOSE_ANSWERS:-/dev/tty}"

names=(default no-platform-input gl mailbox immediate bgra rgba)
envs=("" "MIZU_NO_PLATFORM_INPUT=1" "WGPU_BACKEND=gl" "MIZU_PRESENT_MODE=mailbox"
    "MIZU_PRESENT_MODE=immediate" "MIZU_SURFACE_FORMAT=bgra" "MIZU_SURFACE_FORMAT=rgba")

declare -A seen
total="${#names[@]}"
have_vkcube=0
if command -v vkcube >/dev/null 2>&1; then
    have_vkcube=1
    total=$((total + 1))
fi

ask() {
    local ans=""
    while [ "$ans" != y ] && [ "$ans" != n ]; do
        read -r -p "      Did you see the content (y/n)? " ans <"$answers"
    done
    echo "$ans"
}

echo "mizu-diagnose: $total short runs of $secs s each. A window opens every time;"
echo "just look at it and answer. Nothing is changed on your system."

i=0
for n in "${names[@]}"; do
    e="${envs[$i]}"
    i=$((i + 1))
    printf '\n[%d/%d] %s   (%s)\n' "$i" "$total" "$n" "${e:-no extra settings}"
    # shellcheck disable=SC2086
    env RUST_LOG=mizu=info,wgpu=warn $e timeout "$secs" mizu --diag "$file" >"$out/$n.log" 2>&1
    seen[$n]="$(ask)"
done

if [ "$have_vkcube" = 1 ]; then
    printf '\n[%d/%d] vkcube   (a plain Vulkan test program, not mizu)\n' "$total" "$total"
    timeout "$secs" vkcube >"$out/vkcube.log" 2>&1
    seen[vkcube]="$(ask)"
fi

report="$out/report.txt"
{
    echo "mizu diagnose report"
    echo "kernel:  $(uname -sr)"
    echo "session: type=${XDG_SESSION_TYPE:-?} desktop=${XDG_CURRENT_DESKTOP:-?} WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-} DISPLAY=${DISPLAY:-}"
    echo "mizu:    $(mizu --version 2>&1)"
    first=1
    for n in "${names[@]}"; do
        echo
        echo "== $n: content visible = ${seen[$n]}"
        if [ "$first" = 1 ]; then
            grep -E 'GPU:|surface:|wayland:' "$out/$n.log" | cut -c1-300
            first=0
        fi
        grep -E 'first frame|WARN|ERROR|panicked|diag frame|non-black|completely black|window event' "$out/$n.log" |
            cut -c1-300 | head -n 14
    done
    if [ "$have_vkcube" = 1 ]; then
        echo
        echo "== vkcube: content visible = ${seen[vkcube]}"
        head -n 6 "$out/vkcube.log" | cut -c1-200
    fi
} >"$report"

echo
echo "================ copy everything below this line ================"
cat "$report"
echo "================================================================="
echo "(also saved to $report)"
