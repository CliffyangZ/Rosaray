#!/usr/bin/env bash

# Start Rosaray's local service and frontend, then open the connected app.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
FRONTEND_DIR="$PROJECT_DIR/frontend"
BACKEND_DIR="$PROJECT_DIR/backend"
DATA_DIR="${ROSARAY_PROJECT_DIR:-$BACKEND_DIR/rosaray-project}"
HOST="127.0.0.1"
FRONTEND_PORT="5173"
BACKEND_LOG="$(mktemp)"
BACKEND_PID=""

fail() {
  echo "Rosaray 無法啟動：$*" >&2
  exit 1
}

cleanup() {
  local exit_code=$?
  trap - EXIT
  if [ -n "$BACKEND_PID" ] && kill -0 "$BACKEND_PID" 2>/dev/null; then
    kill "$BACKEND_PID" 2>/dev/null || true
    wait "$BACKEND_PID" 2>/dev/null || true
  fi
  rm -f "$BACKEND_LOG"
  exit "$exit_code"
}
trap cleanup EXIT

if ! command -v node >/dev/null 2>&1 || ! command -v npm >/dev/null 2>&1; then
  fail "請安裝 Node.js 20.19 或以上版本與 npm。"
fi
if ! command -v cargo >/dev/null 2>&1; then
  fail "請安裝 Rust（含 cargo）後再試一次。"
fi

NODE_VERSION="$(node -p 'process.versions.node')"
IFS='.' read -r NODE_MAJOR NODE_MINOR _ <<< "$NODE_VERSION"
if (( NODE_MAJOR < 20 || (NODE_MAJOR == 20 && NODE_MINOR < 19) )); then
  fail "目前 Node.js 版本為 $NODE_VERSION；Rosaray 需要 Node.js 20.19 或以上版本。"
fi

if [ ! -d "$FRONTEND_DIR/node_modules" ]; then
  echo "正在安裝前端相依套件…"
  (cd "$FRONTEND_DIR" && npm ci)
fi

if [ -z "${ROSARAY_PASSPHRASE:-}" ]; then
  if [ ! -t 0 ]; then
    fail "請設定 ROSARAY_PASSPHRASE 環境變數後再以非互動方式啟動。"
  fi
  read -r -s -p "Rosaray 專案密碼：" ROSARAY_PASSPHRASE
  echo
  [ -n "$ROSARAY_PASSPHRASE" ] || fail "專案密碼不可為空。"
  export ROSARAY_PASSPHRASE
fi

echo "正在啟動 Rosaray Local Service…"
(cd "$BACKEND_DIR" && cargo run --locked --quiet -- --project-dir "$DATA_DIR") >"$BACKEND_LOG" 2>&1 &
BACKEND_PID=$!

SERVICE_INFO=""
for _ in {1..1200}; do
  service_line="$(grep -m1 '^{' "$BACKEND_LOG" || true)"
  if [ -n "$service_line" ]; then
    SERVICE_INFO="$(node -e '
      try {
        const service = JSON.parse(process.argv[1]);
        if (!Number.isInteger(service.port) || !/^[0-9a-f]{64}$/.test(service.session_token)) process.exit(1);
        process.stdout.write(`${service.port} ${service.session_token}`);
      } catch { process.exit(1); }
    ' "$service_line")" || true
    [ -n "$SERVICE_INFO" ] && break
  fi
  if ! kill -0 "$BACKEND_PID" 2>/dev/null; then
    cat "$BACKEND_LOG" >&2
    fail "Local Service 已意外結束。"
  fi
  sleep 0.25
done

[ -n "$SERVICE_INFO" ] || fail "等候 Local Service 就緒逾時。"
read -r BACKEND_PORT SESSION_TOKEN <<< "$SERVICE_INFO"
APP_URL="http://${HOST}:${FRONTEND_PORT}/?rosarayPort=${BACKEND_PORT}&rosaraySession=${SESSION_TOKEN}"

open_browser() {
  if [ "${ROSARAY_NO_BROWSER:-}" = "1" ]; then
    echo "瀏覽器自動開啟已略過；工作站網址：$APP_URL"
    return
  fi
  for _ in {1..60}; do
    if curl --silent --fail "http://${HOST}:${FRONTEND_PORT}" >/dev/null 2>&1; then
      if command -v open >/dev/null 2>&1; then
        open "$APP_URL"
      elif command -v xdg-open >/dev/null 2>&1; then
        xdg-open "$APP_URL" >/dev/null 2>&1
      else
        echo "請在瀏覽器開啟：$APP_URL"
      fi
      return
    fi
    sleep 0.5
  done
  echo "前端啟動後，請在瀏覽器開啟：$APP_URL" >&2
}

echo "正在啟動 Rosaray：$APP_URL"
open_browser &

cd "$FRONTEND_DIR"
npm run dev -- --host "$HOST" --port "$FRONTEND_PORT" --strictPort
