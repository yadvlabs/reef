#!/bin/bash
# Download the stock actions/runner for this machine, register it against a Reef repository
# using the developer's token in .dev/token, and run it in the foreground.
# usage: scripts/dev-runner.sh owner/name [site_url] [labels] [name]
set -euo pipefail
REPO=$1
SITE=${2:-http://localhost:4920}
LABELS=${3:-}
NAME=${4:-reef-dev-runner}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
TOKEN_FILE=$ROOT/.dev/token
VERSION=${RUNNER_VERSION:-2.337.0}
DIR=$ROOT/.dev/runner/$(echo "$REPO" | tr '/' '-')

if [ ! -f "$TOKEN_FILE" ]; then
  echo "no API token in $TOKEN_FILE; create one with: target/debug/reef --config .dev/reef.conf create-token <login> --name dev > .dev/token" >&2
  exit 1
fi
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) PLAT=osx-arm64 ;;
  Darwin-x86_64) PLAT=osx-x64 ;;
  Linux-aarch64) PLAT=linux-arm64 ;;
  Linux-x86_64) PLAT=linux-x64 ;;
  *) echo "unsupported platform $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac
TARBALL=$ROOT/.dev/runner/actions-runner-$PLAT-$VERSION.tar.gz
mkdir -p "$DIR"
if [ ! -f "$TARBALL" ]; then
  echo "downloading actions/runner $VERSION for $PLAT"
  curl -fsSL -o "$TARBALL" "https://github.com/actions/runner/releases/download/v$VERSION/actions-runner-$PLAT-$VERSION.tar.gz"
fi
if [ ! -f "$DIR/config.sh" ]; then
  tar xzf "$TARBALL" -C "$DIR"
fi
API=${SITE%/}/api/v3
# The runner keeps only the host of --url (it drops any port), so register against the
# portless origin and make sure Reef answers there (make dev-runners adds a listener on :80).
RUNNER_URL=$(echo "${SITE%/}" | sed -E 's#^(https?://[^/:]+)(:[0-9]+)?$#\1#')
if ! curl -fsS -m 2 -o /dev/null "$RUNNER_URL/healthz" 2>/dev/null; then
  echo "reef is not answering on $RUNNER_URL; stock runners need it on port 80/443. Start it with 'make dev-runners'." >&2
  exit 1
fi
REG=$(curl -fsS -X POST -H "Authorization: token $(cat "$TOKEN_FILE")" "$API/repos/$REPO/actions/runners/registration-token" | python3 -c 'import json,sys;print(json.load(sys.stdin)["token"])')
cd "$DIR"
if [ ! -f .runner ]; then
  ./config.sh --unattended --replace --url "$RUNNER_URL/$REPO" --token "$REG" --name "$NAME" ${LABELS:+--labels "$LABELS"}
fi
echo "runner '$NAME' registered to $REPO; running (Ctrl+C to stop, it stays registered)"
exec ./run.sh
