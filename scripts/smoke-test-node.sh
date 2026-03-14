#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 4 ]]; then
  echo "Usage: $0 <server_url> <api_token> <session_id> <client_id>"
  exit 1
fi

SERVER_URL="$1"
API_TOKEN="$2"
SESSION_ID="$3"
CLIENT_ID="$4"

bind() {
  curl -sS \
    -H "Authorization: Bearer $API_TOKEN" \
    -H 'Content-Type: application/json' \
    -X POST \
    -d "{\"client_id\":\"$CLIENT_ID\",\"device_mode\":\"manual\"}" \
    "$SERVER_URL/agent/v1/sessions/$SESSION_ID/bind-device"
}

dispatch_php() {
  local action="$1"
  local payload_php="$2"
  php -r "require '/Volumes/Web/duxai/vendor/autoload.php'; Core\\App::create(basePath: '/Volumes/Web/duxai', debug: true, timezone: 'UTC'); \
  \$s = App\\Ai\\Models\\AiAgentSession::query()->with(['agent','device'])->find($SESSION_ID); \
  var_export(App\\AiDesktop\\Service\\Desktop\\DispatchService::dispatch(\$s, '$action', $payload_php, ['await'=>'sync','timeout'=>30]));"
}

echo '== bind session =='
bind

echo '
== system.info =='
dispatch_php "system.info" '[]'

echo '
== browser.read =='
dispatch_php "browser.read" "['url'=>'https://www.v2ex.com/?tab=hot']"

echo '
== screen.capture =='
dispatch_php "screen.capture" '[]'

echo '
== browser.screenshot =='
dispatch_php "browser.screenshot" "['full_page'=>false]"
