#!/usr/bin/env bash
# Сквозная проверка страницы моста для Claude (TicketBoard/Bridge) в headless Chromium: настоящий мост и разбор ответов
# Интрасервиса (SelfCheck в режиме bridge), поддельный Интрасервис (fake-intraservice.mjs), подменённый api.anthropic.com.
# Нужны .NET 10 SDK, Node и Playwright с Chromium (в контейнере агента уже есть). Порты 47822 и 47899 должны быть свободны.
set -euo pipefail
cd "$(dirname "$0")"
dotnet build .. -c Debug -v quiet -nologo > /dev/null
KEY=$(node -e 'console.log(require("crypto").randomBytes(16).toString("hex"))')
node fake-intraservice.mjs > /dev/null & FAKE=$!
dotnet ../bin/Debug/net10.0/TicketBoard.SelfCheck.dll bridge "$KEY" > /dev/null & HOST=$!
trap 'kill $FAKE $HOST 2>/dev/null || true' EXIT
for _ in $(seq 60); do curl -sf -o /dev/null http://127.0.0.1:47822/ && break; sleep 0.5; done
for scheme in light dark; do NODE_PATH="$(npm root -g)" node check.cjs "$KEY" "$scheme"; done
