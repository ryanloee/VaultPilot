#!/usr/bin/env bash
# Full functional smoke test for vaultpilot-cli.
# Exercises all non-API subcommands. No API key required.
# Exits 0 on success, 1 on failure.

set -euo pipefail

CLI="${1:?Usage: smoke-test-linux-cli.sh <path-to-vaultpilot-cli>}"
WORKDIR=$(mktemp -d)
trap 'rm -rf "$WORKDIR"' EXIT

VAULT="$WORKDIR/vault"
mkdir -p "$VAULT"

PASS=0
FAIL=0

run_test() {
  local name="$1"
  shift
  echo -n "  $name ... "
  if output=$("$CLI" --vault-dir "$VAULT" "$@" 2>&1); then
    echo "OK"
    PASS=$((PASS + 1))
  else
    echo "FAIL (exit $?)"
    echo "    output: $output"
    FAIL=$((FAIL + 1))
  fi
}

run_check() {
  local name="$1"
  shift
  echo -n "  $name ... "
  if output=$("$@" 2>&1); then
    echo "OK"
    PASS=$((PASS + 1))
  else
    echo "FAIL (exit $?)"
    echo "    output: $output"
    FAIL=$((FAIL + 1))
  fi
}

echo "=== VaultPilot CLI Full Smoke Test ==="
echo "CLI: $CLI"
echo "Vault: $VAULT"
echo ""

# ── init ──
echo "[init]"
run_test "init vault" init
run_check "vault dir created" test -d "$VAULT"

# ── settings ──
echo "[settings]"
run_test "settings get" settings get --pretty

# ── notes CRUD ──
echo "[notes]"

# Create 3 notes with correct NoteDocument format
for i in 1 2 3; do
  "$CLI" --vault-dir "$VAULT" notes create <<< \
    "{\"meta\":{\"id\":\"n$i\",\"title\":\"Note $i\",\"tags\":[\"test\",\"ci\"]},\"body\":\"Body of note $i\"}" \
    > /dev/null 2>&1
done

run_check "notes list (3 notes)" bash -c '
  count=$("'$CLI'" --vault-dir "'"$VAULT"'" notes list --pretty \
    | python3 -c "import json,sys; d=json.load(sys.stdin); print(d[\"total\"])")
  [ "$count" = "3" ]
'

run_check "notes get by id returns correct title" bash -c '
  "'"$CLI"'" --vault-dir "'"$VAULT"'" notes get n1 --pretty \
    | python3 -c "import json,sys; d=json.load(sys.stdin); assert d[\"meta\"][\"title\"]==\"Note 1\",d"
'

run_check "notes get body content" bash -c '
  "'"$CLI"'" --vault-dir "'"$VAULT"'" notes get n2 --pretty \
    | python3 -c "import json,sys; d=json.load(sys.stdin); assert \"Body of note 2\" in d[\"body\"],d"
'

run_check "notes search finds match" bash -c '
  "'"$CLI"'" --vault-dir "'"$VAULT"'" notes search --query "Note 2" --pretty \
    | python3 -c "import json,sys; d=json.load(sys.stdin); assert d[\"total\"]>=1,d"
'

run_check "notes search by tag" bash -c '
  "'"$CLI"'" --vault-dir "'"$VAULT"'" notes search --query "" --tags "test" --pretty \
    | python3 -c "import json,sys; d=json.load(sys.stdin); assert d[\"total\"]>=1,d"
'

run_check "notes update (re-create with new body)" bash -c '
  "'"$CLI"'" --vault-dir "'"$VAULT"'" notes create <<< \
    "{\"meta\":{\"id\":\"n1\",\"title\":\"Note 1 Updated\"},\"body\":\"Updated body\"}"
  got=$("'$CLI'" --vault-dir "'"$VAULT"'" notes get n1 --pretty)
  echo "$got" | python3 -c "import json,sys; d=json.load(sys.stdin); assert d[\"meta\"][\"title\"]==\"Note 1 Updated\""
'

run_check "notes delete" bash -c '
  "'"$CLI"'" --vault-dir "'"$VAULT"'" notes delete n3
  count=$("'$CLI'" --vault-dir "'"$VAULT"'" notes list --pretty \
    | python3 -c "import json,sys; d=json.load(sys.stdin); print(d[\"total\"])")
  [ "$count" = "2" ]
'

# ── import / export ──
echo "[import/export]"

# Create a markdown file and import it
mkdir -p "$WORKDIR/import"
echo -e "# Imported Note\n\nThis was imported from a file." > "$WORKDIR/import/test.md"

run_check "notes import markdown" bash -c '
  "'"$CLI"'" --vault-dir "'"$VAULT"'" notes import "'"$WORKDIR"'/import/test.md"
  count=$("'$CLI'" --vault-dir "'"$VAULT"'" notes list --pretty \
    | python3 -c "import json,sys; d=json.load(sys.stdin); print(d[\"total\"])")
  [ "$count" -ge 3 ]
'

run_check "vault export zip" bash -c '
  "'"$CLI"'" --vault-dir "'"$VAULT"'" vault export --output "'"$WORKDIR"'/export.zip"
  test -f "'"$WORKDIR"'/export.zip"
  python3 -c "import zipfile; z=zipfile.ZipFile(\"'"$WORKDIR"'/export.zip\"); assert len(z.namelist())>0"
'

# ── index ──
echo "[index]"
run_test "index rebuild" index rebuild

# ── serve (start, check HTTP, stop) ──
echo "[serve]"
run_check "serve starts and responds to /v1/models" bash -c '
  "'"$CLI"'" --vault-dir "'"$VAULT"'" serve --port 19876 &
  PID=$!
  sleep 3
  RESP=$(curl -sf http://localhost:19876/v1/models 2>&1)
  kill $PID 2>/dev/null || true
  wait $PID 2>/dev/null || true
  echo "$RESP" | grep -q "data"
'

# ── mcp (start, send initialize, check response) ──
echo "[mcp]"
run_check "mcp responds to initialize" bash -c '
  RESP=$(echo "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"capabilities\":{}}}" \
    | timeout 5 "'"$CLI"'" --vault-dir "'"$VAULT"'" mcp 2>/dev/null || true)
  echo "$RESP" | grep -q "jsonrpc"
'

# ── help (exercises arg parsing for all subcommands) ──
echo "[help]"
for subcmd in init serve chat settings notes index ask compress vault mcp; do
  run_test "$subcmd --help" "$subcmd" --help
done

# ── summary ──
echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="

if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
