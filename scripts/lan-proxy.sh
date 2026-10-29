#!/bin/sh
# HTTPS reverse proxy so another machine on the LAN can open the game.
# `make dev` / `make run` keep serving http://127.0.0.1. This only publishes
# an nginx container in front of that port. WebTransport to the match is UDP
# on the host (ports 7000-7099) and is not proxied.
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
listen=${LISTEN:-127.0.0.1:8080}
lan_port=${LAN_PORT:-8443}
upstream_port=${listen##*:}
image=${LAN_IMAGE:-nginx:1.27-alpine}

need() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "make lan needs $1" >&2
        exit 1
    }
}
need docker
need openssl
need python3

if [ -n "${LAN_IP:-}" ]; then
    ip=$LAN_IP
else
    ip=$(python3 - <<'PY'
import re, socket, subprocess, sys

def private(ip):
    try:
        a, b, *_ = (int(x) for x in ip.split("."))
    except ValueError:
        return False
    return a == 10 or (a == 192 and b == 168) or (a == 172 and 16 <= b <= 31)

def emit(ip):
    if private(ip):
        print(ip)
        raise SystemExit

# A proxy VPN often owns the default route (198.18.0.0/15). Skip anything
# that is not an RFC1918 address, then prefer the Mac's primary interface.
try:
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.connect(("8.8.8.8", 80))
    emit(sock.getsockname()[0])
    sock.close()
except OSError:
    pass

if sys.platform == "darwin":
    for iface in ("en0", "en1"):
        try:
            out = subprocess.check_output(
                ["ipconfig", "getifaddr", iface], text=True, stderr=subprocess.DEVNULL
            )
        except (OSError, subprocess.CalledProcessError):
            continue
        emit(out.strip())

try:
    out = subprocess.check_output(["ifconfig"], text=True, stderr=subprocess.DEVNULL)
except (OSError, subprocess.CalledProcessError):
    out = ""
for found in re.findall(r"\binet (\d+\.\d+\.\d+\.\d+)", out):
    emit(found)
raise SystemExit(1)
PY
    ) || {
        echo "could not detect a LAN address; rerun as: make lan LAN_IP=192.168.1.10" >&2
        exit 1
    }
fi

mkdir -p "$root/target/lan"
# Regenerated every start so the certificate names this machine's current address.
openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout "$root/target/lan/key.pem" -out "$root/target/lan/cert.pem" -days 14 \
    -subj "/CN=evoclash-lan" \
    -addext "subjectAltName=DNS:localhost,IP:127.0.0.1,IP:${ip}" \
    >/dev/null 2>&1
chmod a+r "$root/target/lan/cert.pem" "$root/target/lan/key.pem"

echo "→ other machines: https://${ip}:${lan_port}  (accept the self-signed certificate once)"
echo "→ this machine stays on http://127.0.0.1:${upstream_port}"
echo "→ the matchmaker must advertise ${ip}, or join tokens point at 127.0.0.1:"
echo "    make dev PUBLIC_HOST=${ip}"
echo "→ Ctrl-C stops the proxy"

docker rm -f evoclash-lan >/dev/null 2>&1 || true
exec docker run --rm --name evoclash-lan \
    -p "${lan_port}:8443" \
    --add-host=host.docker.internal:host-gateway \
    -e "MATCHMAKER_UPSTREAM=host.docker.internal:${upstream_port}" \
    -v "${root}/deploy/lan/default.conf.template:/etc/nginx/templates/default.conf.template:ro" \
    -v "${root}/target/lan:/etc/nginx/certs:ro" \
    "$image"
