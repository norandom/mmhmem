#!/bin/sh
# Offline checks of the rendered install.sh under one POSIX shell.
# Usage: test-bootstrap-sh.sh <shell> <fixture-dir> <port>
# The fixture (built by scripts/test-bootstraps.ps1) holds a local HTTP server
# log, the rendered scripts and the served archives; uname is stubbed so the
# Linux and macOS code paths both run on any host. Not covered here: a real
# /dev/tty, real macOS tools (shasum, sysctl), wget.
SH=$1
FIX=$2
PORT=$3
TAG=v1.2.3
WORK=$(mktemp -d "${TMPDIR:-/tmp}/bootstrap-test.XXXXXX") || exit 2
FAILS=0
CASES=0
trap 'rm -rf "$WORK"' EXIT

pass() { CASES=$((CASES + 1)); }
bad() { CASES=$((CASES + 1)); FAILS=$((FAILS + 1)); printf 'FAIL [%s]: %s\n' "$SH" "$*"; }
expect() { # description condition-exit-status
    if [ "$2" -eq 0 ]; then pass; else bad "$1"; fi
}
contains() { grep -F -q -- "$2" "$1"; }

# Stub uname: $1 = os, $2 = cpu.
make_uname() {
    mkdir -p "$WORK/bin"
    cat > "$WORK/bin/uname" <<STUB
#!/bin/sh
case "\$1" in -s) echo '$1' ;; -m) echo '$2' ;; *) echo '$1' ;; esac
STUB
    chmod +x "$WORK/bin/uname"
}

# run <script> <exit-code-for-fake> : sets OUT (file), RC. Uses a private TMPDIR.
run() {
    rm -rf "$WORK/tmp"; rm -f "$WORK"/fake.*; mkdir -p "$WORK/tmp"
    : > "$WORK/out.txt"
    LOGLINES=$(wc -l < "$FIX/server.log")
    PATH="$WORK/bin:$PATH" TMPDIR="$WORK/tmp" FAKE_LOG="$WORK/fake" FAKE_EXIT="$2" \
        "$SH" "$1" > "$WORK/out.txt" 2>&1 < /dev/null
    RC=$?
}
new_requests() { tail -n +"$((LOGLINES + 1))" "$FIX/server.log"; }
tmp_clean() { [ -z "$(ls -A "$WORK/tmp")" ]; }

# 1. Happy path per target, exit code passed through.
for case in "Linux x86_64 x86_64-unknown-linux-musl" "Darwin arm64 aarch64-apple-darwin" "Darwin x86_64 x86_64-apple-darwin"; do
    set -- $case
    make_uname "$1" "$2"
    for code in 0 7; do
        run "$FIX/ok/install.sh" "$code"
        [ "$RC" -eq "$code" ]; expect "$3 exit $code passed through (got $RC)" $?
        new_requests | grep -F -q "mmhmem-$TAG-$3.tar.gz"; expect "$3 requested its own archive" $?
        [ "$(cat "$WORK/fake.args" 2>/dev/null)" = "setup
--bootstrap
$TAG" ]; expect "$3 setup called with exactly 'setup --bootstrap $TAG'" $?
        case "$(cat "$WORK/fake.self" 2>/dev/null)" in "$WORK/tmp/"*"/mmhmem") pass ;; *) bad "$3 fake ran from temp folder: $(cat "$WORK/fake.self")" ;; esac
        [ "$(cat "$WORK/fake.stdin" 2>/dev/null)" = "notty" ]; expect "$3 stdin is not the script when no tty" $?
        tmp_clean; expect "$3 temp folder removed (exit $code)" $?
        contains "$WORK/out.txt" "SHA-256 matches"; expect "$3 says the download was checked" $?
    done
done

# 2. Flags are ignored, never forwarded.
make_uname Linux x86_64
run "$FIX/ok/install.sh" 0
"$SH" -c "PATH='$WORK/bin':\$PATH TMPDIR='$WORK/tmp' FAKE_LOG='$WORK/fake' '$SH' '$FIX/ok/install.sh' --agents claude --yes" > /dev/null 2>&1 < /dev/null
[ "$(cat "$WORK/fake.args")" = "setup
--bootstrap
$TAG" ]; expect "extra flags are not forwarded" $?

# 3. curl | sh style: the script arrives on stdin.
rm -rf "$WORK/tmp"; rm -f "$WORK"/fake.*; mkdir -p "$WORK/tmp"
cat "$FIX/ok/install.sh" | PATH="$WORK/bin:$PATH" TMPDIR="$WORK/tmp" FAKE_LOG="$WORK/fake" FAKE_EXIT=0 "$SH" > "$WORK/out.txt" 2>&1
RC=$?
[ "$RC" -eq 0 ] && [ -f "$WORK/fake.args" ]; expect "piped script ran setup (rc=$RC)" $?
tmp_clean; expect "piped run removed the temp folder" $?

# 4. Hash mismatch.
run "$FIX/badhash/install.sh" 0
[ "$RC" -ne 0 ]; expect "mismatch exits non-zero" $?
[ ! -e "$WORK/fake.args" ]; expect "mismatch never runs setup" $?
contains "$WORK/out.txt" "expected SHA-256: 0000000000000000000000000000000000000000000000000000000000000000"; expect "mismatch shows expected hash" $?
contains "$WORK/out.txt" "actual SHA-256:"; expect "mismatch shows actual hash" $?
contains "$WORK/out.txt" "Nothing was installed or changed"; expect "mismatch says nothing changed" $?
tmp_clean; expect "mismatch removed the temp folder" $?

# 5. 404 on the archive.
run "$FIX/missing/install.sh" 0
[ "$RC" -ne 0 ]; expect "404 exits non-zero" $?
[ ! -e "$WORK/fake.args" ]; expect "404 never runs setup" $?
contains "$WORK/out.txt" "download failed"; expect "404 says the download failed" $?
contains "$WORK/out.txt" "Nothing was installed or changed"; expect "404 says nothing changed" $?
tmp_clean; expect "404 removed the temp folder" $?

# 6. Unsupported platforms: plain message, nothing downloaded, no temp folder.
for case in "FreeBSD amd64" "Linux aarch64" "Linux armv7l" "Darwin ppc"; do
    set -- $case
    make_uname "$1" "$2"
    run "$FIX/ok/install.sh" 0
    [ "$RC" -ne 0 ]; expect "$1 $2 exits non-zero" $?
    contains "$WORK/out.txt" "does not have a download for $1 on $2"; expect "$1 $2 says unsupported" $?
    [ -z "$(new_requests)" ]; expect "$1 $2 downloaded nothing" $?
    [ ! -e "$WORK/fake.args" ]; expect "$1 $2 never runs setup" $?
    tmp_clean; expect "$1 $2 created no temp folder" $?
done

# 7. Static checks on the rendered script.
if grep -E -i 'claude|codex|opencode|antigravity|cursor|getopts|--agents' "$FIX/ok/install.sh" > /dev/null; then bad "script mentions agents or parses options"; else pass; fi
if grep -E '@@' "$FIX/ok/install.sh" > /dev/null; then bad "script has a leftover placeholder"; else pass; fi
if grep -E 'PATH=|\.bashrc|\.profile|\.zshrc' "$FIX/ok/install.sh" > /dev/null; then bad "script touches PATH or profiles"; else pass; fi

printf '%s: %s checks, %s failed (port %s)\n' "$SH" "$CASES" "$FAILS" "$PORT"
[ "$FAILS" -eq 0 ]
