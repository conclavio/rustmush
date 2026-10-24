#!/usr/bin/env bash
set -euo pipefail
ROOT=/opt/rustmush/dev1
[ "$(realpath "$ROOT")" = /opt/rustmush/dev1 ]
EVIDENCE="$ROOT/r10-evidence"
export PATH="$ROOT/r1-tools/node_modules/node/bin:/root/.cargo/bin:$PATH"
export NODE_EXTRA_CA_CERTS="$ROOT/tls/cert.pem"
grep -q '^R10_BUILD_COMPLETE$' "$ROOT/r10-build.log"
cmp "$ROOT/build/.source-tree" "$ROOT/r10-source-tree.txt"
test -f "$EVIDENCE/process.json"
test -f "$EVIDENCE/fresh.json"
test -f "$EVIDENCE/snapshot.json"
systemctl start dev1mush-gateway
for ((i=0;i<90;i++)); do
 if curl -fsS --cacert "$ROOT/tls/cert.pem" https://conclav.io:6790/ >/dev/null 2>&1; then break; fi
 sleep 1
done
node "$ROOT/build/scripts/relay-r10-live.mjs" https://conclav.io:6790 "$ROOT/r1-evidence/credentials.json" "$EVIDENCE" 6788 > "$EVIDENCE/live-split.log" 2>&1
cp "$EVIDENCE/live.json" "$EVIDENCE/live-split.json"
systemctl stop dev1mush-gateway
[ "$(systemctl show dev1mush-gateway -p MainPID --value)" = 0 ]
systemd-run --unit=rustmush-r10-allinone --property=User=mush --property=Group=mush --property=WorkingDirectory="$ROOT/build" --property=Restart=always --property=RestartSec=2 "$ROOT/build/target/release/mushd" --config "$ROOT/mushd.toml"
for ((i=0;i<90;i++)); do
 if curl -fsS --cacert "$ROOT/tls/cert.pem" https://conclav.io:6790/ >/dev/null 2>&1; then break; fi
 sleep 1
done
RELAY_GATE_MODE=all-in-one node "$ROOT/build/scripts/relay-r10-live.mjs" https://conclav.io:6790 "$ROOT/r1-evidence/credentials.json" "$EVIDENCE" 6788 > "$EVIDENCE/live-allinone.log" 2>&1
cp "$EVIDENCE/live.json" "$EVIDENCE/live-allinone.json"
python3 "$ROOT/build/scripts/relay-r10-ssh.py" > "$EVIDENCE/ssh.log" 2>&1
systemctl stop rustmush-r10-allinone
python3 "$ROOT/build/scripts/relay-r10-world-fixture.py" restore
systemctl start dev1mush-gateway
for ((i=0;i<90;i++)); do
 if curl -fsS --cacert "$ROOT/tls/cert.pem" https://conclav.io:6790/ >/dev/null 2>&1; then break; fi
 sleep 1
done
systemctl is-active dev1mush-gateway redis-dev1mush
systemctl show dev2mush-gateway -p MainPID > "$EVIDENCE/dev2-after.txt"
cmp "$EVIDENCE/dev2-before.txt" "$EVIDENCE/dev2-after.txt"
sha256sum "$ROOT/build/target/release/mushd" "$ROOT/build/target/release/mush-server" "$ROOT/build/target/release/mush-gateway" > "$EVIDENCE/binary-hashes.txt"
echo R10_LIVE_PASSED
