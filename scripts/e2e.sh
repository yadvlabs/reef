#!/usr/bin/env bash
# End-to-end smoke test of the basic feature set against a throwaway server: sign-up, API
# tokens, repositories, git over HTTP and SSH, issues, pull requests and merges, releases,
# wikis, organizations, GraphQL, Actions endpoints, metrics. Needs the debug binary (or
# BIN=...), git, ssh, ssh-keygen and python3. Usage: scripts/e2e.sh [--keep]
set -uo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
BIN=${BIN:-$ROOT/target/debug/reef}
PORT=${E2E_PORT:-4941}
SSH_PORT=${E2E_SSH_PORT:-4942}
HOST=127.0.0.1
URL="http://$HOST:$PORT"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/reef-e2e.XXXXXX")
KEEP=${1:-}
PASS=0; FAIL=0; SERVER=

cleanup() {
  [ -n "$SERVER" ] && kill "$SERVER" 2>/dev/null && wait "$SERVER" 2>/dev/null
  if [ "$FAIL" -gt 0 ] || [ "$KEEP" = "--keep" ]; then echo "work dir kept: $WORK (server log: $WORK/server.log)"; else rm -rf "$WORK"; fi
}
trap cleanup EXIT

j() { # j '.a.b.0' reads a JSON path from stdin; booleans print as true/false, null as null
  python3 -I -c '
import json, sys
d = json.load(sys.stdin)
for k in sys.argv[1].split("."):
    if k == "": continue
    d = d[int(k)] if isinstance(d, list) else d[k]
if isinstance(d, bool): print(str(d).lower())
elif d is None: print("null")
else: print(d)' "$1" 2>/dev/null || echo "<no json>"
}
expect() { # expect "what" actual wanted
  if [ "$2" = "$3" ]; then PASS=$((PASS + 1)); echo "  ok   $1"; else FAIL=$((FAIL + 1)); echo "  FAIL $1: got '$2', wanted '$3'"; fi
}
api() { curl -sS -H "Authorization: token $TOKEN" -H "Accept: application/vnd.github+json" -H "Content-Type: application/json" "$@"; }
status() { curl -sS -o /dev/null -w '%{http_code}' "$@"; }
gitc() { git -c user.name=E2E -c user.email=e2e@example.com -c init.defaultBranch=main "$@"; }

[ -x "$BIN" ] || { echo "no binary at $BIN; run: cargo build --bin reef (or make dev-build)"; exit 1; }
cat > "$WORK/reef.conf" <<CONF
# TOML
listen = "$HOST:$PORT"
site_url = "$URL"
ssh_listen = "$HOST:$SSH_PORT"
data_dir = "$WORK/data"
node_id = 3
allow_signup = true
[log]
format = "text"
CONF
"$BIN" serve --config "$WORK/reef.conf" > "$WORK/server.log" 2>&1 &
SERVER=$!
for _ in $(seq 1 120); do curl -sf -o /dev/null "$URL/readyz" && break; sleep 0.25; done
curl -sf -o /dev/null "$URL/readyz" || { echo "server did not become ready"; tail -20 "$WORK/server.log"; exit 1; }
echo "server $($BIN version) at $URL (ssh $SSH_PORT)"

echo "health"
expect "healthz" "$(status "$URL/healthz")" 200
expect "readyz" "$(curl -s "$URL/readyz" | j .status)" ok
expect "metrics carry the build" "$(curl -s "$URL/metrics" | grep -c '^reef_build_info')" 1

echo "accounts and tokens"
SIGNUP=$(curl -sS -H 'Content-Type: application/json' -d '{"login":"rohit","email":"rohit@example.com","password":"correct horse battery","name":"Rohit"}' "$URL/api/reef/signup")
expect "first sign-up is the site administrator" "$(echo "$SIGNUP" | j .site_admin)" true
TOKEN=$("$BIN" create-token rohit --name e2e --config "$WORK/reef.conf" 2>/dev/null | tail -1)
expect "a personal access token was minted" "${TOKEN:0:4}" "fgp_"
expect "GET /user with the token" "$(api "$URL/api/v3/user" | j .login)" rohit
expect "rate limit endpoint" "$(status -H "Authorization: token $TOKEN" "$URL/api/v3/rate_limit")" 200
expect "GraphQL viewer" "$(api -d '{"query":"{ viewer { login } }"}' "$URL/api/graphql" | j .data.viewer.login)" rohit

echo "repositories and git over HTTP"
expect "create a repository" "$(api -d '{"name":"demo","description":"e2e"}' "$URL/api/v3/user/repos" | j .full_name)" rohit/demo
gitc init -q "$WORK/src" && (cd "$WORK/src" && echo "# demo" > README.md && gitc add . && gitc commit -q -m "initial" && gitc push -q "http://rohit:$TOKEN@$HOST:$PORT/rohit/demo.git" main) 2>"$WORK/push.err"
expect "push over HTTP" "$?" 0
expect "the branch is listed" "$(api "$URL/api/v3/repos/rohit/demo/branches" | j .0.name)" main
expect "contents API decodes the file" "$(api "$URL/api/v3/repos/rohit/demo/contents/README.md" | j .content | base64 -d | tr -d '\n')" "# demo"
expect "raw file" "$(curl -s -H "Authorization: token $TOKEN" "$URL/rohit/demo/raw/main/README.md" | tr -d '\n')" "# demo"
gitc clone -q "http://rohit:$TOKEN@$HOST:$PORT/rohit/demo.git" "$WORK/clone-http" 2>/dev/null
expect "clone over HTTP" "$(cat "$WORK/clone-http/README.md" 2>/dev/null)" "# demo"

echo "issues"
expect "open an issue" "$(api -d '{"title":"Something is off","body":"details"}' "$URL/api/v3/repos/rohit/demo/issues" | j .number)" 1
expect "comment on it" "$(api -d '{"body":"looking"}' "$URL/api/v3/repos/rohit/demo/issues/1/comments" | j .body)" looking
expect "close it" "$(api -X PATCH -d '{"state":"closed"}' "$URL/api/v3/repos/rohit/demo/issues/1" | j .state)" closed
expect "closed issues are listed" "$(api "$URL/api/v3/repos/rohit/demo/issues?state=closed" | j .0.number)" 1

echo "pull requests"
(cd "$WORK/src" && gitc checkout -q -b feature && echo "more" >> README.md && gitc commit -q -am "feature" && gitc push -q "http://rohit:$TOKEN@$HOST:$PORT/rohit/demo.git" feature) 2>>"$WORK/push.err"
expect "push a feature branch" "$?" 0
PR=$(api -d '{"title":"Add more","head":"feature","base":"main","body":"please"}' "$URL/api/v3/repos/rohit/demo/pulls")
expect "open a pull request" "$(echo "$PR" | j .number)" 2
expect "it is mergeable" "$(api "$URL/api/v3/repos/rohit/demo/pulls/2" | j .mergeable)" true
expect "merge it" "$(api -X PUT -d '{}' "$URL/api/v3/repos/rohit/demo/pulls/2/merge" | j .merged)" true
expect "it shows as merged" "$(api "$URL/api/v3/repos/rohit/demo/pulls/2" | j .merged)" true
expect "main now has the change" "$(api "$URL/api/v3/repos/rohit/demo/contents/README.md?ref=main" | j .content | base64 -d | tail -1)" more

echo "releases and tags"
expect "publish a release" "$(api -d '{"tag_name":"v0.0.1","name":"First","target_commitish":"main"}' "$URL/api/v3/repos/rohit/demo/releases" | j .tag_name)" v0.0.1
expect "the tag exists" "$(api "$URL/api/v3/repos/rohit/demo/tags" | j .0.name)" v0.0.1
expect "release by tag" "$(api "$URL/api/v3/repos/rohit/demo/releases/tags/v0.0.1" | j .name)" First
expect "source tarball" "$(status -H "Authorization: token $TOKEN" -L "$URL/api/v3/repos/rohit/demo/tarball/main")" 200

echo "wiki"
expect "create a wiki page" "$(api -d '{"title":"Home","content":"# Welcome","message":"init"}' "$URL/api/reef/repos/rohit/demo/wiki" | j .title)" Home
expect "read it back" "$(api "$URL/api/reef/repos/rohit/demo/wiki/Home" | j .content | tr -d '\n')" "# Welcome"

echo "git over SSH"
ssh-keygen -q -t ed25519 -N '' -f "$WORK/id_e2e"
api -d "{\"title\":\"e2e\",\"key\":\"$(cut -d' ' -f1,2 "$WORK/id_e2e.pub")\"}" "$URL/api/v3/user/keys" > "$WORK/key.json"
expect "register an SSH key" "$(j .title < "$WORK/key.json")" e2e
export GIT_SSH_COMMAND="ssh -i $WORK/id_e2e -o IdentitiesOnly=yes -o PreferredAuthentications=publickey -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -p $SSH_PORT"
gitc clone -q "ssh://git@$HOST:$SSH_PORT/rohit/demo.git" "$WORK/clone-ssh" 2>"$WORK/ssh.err"
expect "clone over SSH" "$(head -1 "$WORK/clone-ssh/README.md" 2>/dev/null)" "# demo"
[ -d "$WORK/clone-ssh" ] || { echo "  ssh said:"; sed 's/^/    /' "$WORK/ssh.err"; echo "  key registration answered:"; sed 's/^/    /' "$WORK/key.json"; echo; }
(cd "$WORK/clone-ssh" && echo "ssh" > SSH.md && gitc add SSH.md && gitc commit -q -m "over ssh" && gitc push -q origin main) 2>>"$WORK/ssh.err"
expect "push over SSH" "$?" 0
expect "the SSH push is visible over the API" "$(api "$URL/api/v3/repos/rohit/demo/contents/SSH.md" | j .content | base64 -d | tr -d '\n')" ssh
unset GIT_SSH_COMMAND

echo "organizations"
expect "create an organization" "$(api -d '{"login":"acme","name":"Acme"}' "$URL/api/reef/orgs" | j .login)" acme
expect "GET /orgs/acme" "$(api "$URL/api/v3/orgs/acme" | j .login)" acme
expect "create a repository in it" "$(api -d '{"name":"lib"}' "$URL/api/v3/orgs/acme/repos" | j .full_name)" acme/lib
expect "the member sees it" "$(api "$URL/api/v3/orgs/acme/repos" | j .0.name)" lib

echo "actions"
expect "no runs yet" "$(api "$URL/api/v3/repos/rohit/demo/actions/runs" | j .total_count)" 0
expect "runner registration token" "$(api -X POST "$URL/api/v3/repos/rohit/demo/actions/runners/registration-token" | j .token | wc -c | tr -d ' ' | awk '{print ($1 > 10) ? "long" : "short"}')" long

echo "access control"
expect "anonymous can read a public repo" "$(status "$URL/api/v3/repos/rohit/demo")" 200
expect "create a private repository" "$(api -d '{"name":"secret","private":true}' "$URL/api/v3/user/repos" | j .private)" true
expect "anonymous cannot see it" "$(status "$URL/api/v3/repos/rohit/secret")" 404
expect "its owner can" "$(api "$URL/api/v3/repos/rohit/secret" | j .full_name)" rohit/secret
expect "a bad token is refused" "$(status -H 'Authorization: token fgp_nope' "$URL/api/v3/user")" 401

echo
echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
