#!/usr/bin/env python3
"""Forward the matchmaker's port on this machine's LAN address.

`make dev` keeps listening on 127.0.0.1. Binding the same port on the LAN
address does not collide with that. GNU nc's `-L` tunnel only carries one
connection, and a browser opens several, so this accepts many at once.
"""

import os
import re
import socket
import subprocess
import sys
import threading

def private(ip):
    try:
        a, b, *_ = (int(part) for part in ip.split("."))
    except ValueError:
        return False
    return a == 10 or (a == 192 and b == 168) or (a == 172 and 16 <= b <= 31)


def lan_ip():
    chosen = os.environ.get("LAN_IP", "").strip()
    if chosen:
        return chosen
    found = []
    try:
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.connect(("8.8.8.8", 80))
        found.append(sock.getsockname()[0])
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
            found.append(out.strip())
    try:
        out = subprocess.check_output(["ifconfig"], text=True, stderr=subprocess.DEVNULL)
    except (OSError, subprocess.CalledProcessError):
        out = ""
    found.extend(re.findall(r"\binet (\d+\.\d+\.\d+\.\d+)", out))
    for ip in found:
        if private(ip):
            return ip
    sys.exit("could not detect a LAN address; rerun as: make lan LAN_IP=192.168.1.10")


def relay(src, dst):
    try:
        while True:
            data = src.recv(65536)
            if not data:
                break
            dst.sendall(data)
    except OSError:
        pass
    finally:
        for sock, how in ((src, socket.SHUT_RD), (dst, socket.SHUT_WR)):
            try:
                sock.shutdown(how)
            except OSError:
                pass


def handle(client, target):
    try:
        upstream = socket.create_connection(target)
    except OSError as err:
        print(f"upstream {target[0]}:{target[1]}: {err}", file=sys.stderr)
        client.close()
        return
    left = threading.Thread(target=relay, args=(client, upstream), daemon=True)
    right = threading.Thread(target=relay, args=(upstream, client), daemon=True)
    left.start()
    right.start()
    left.join()
    right.join()
    client.close()
    upstream.close()


def main():
    listen = os.environ.get("LISTEN", "127.0.0.1:8080")
    port = int(listen.rsplit(":", 1)[-1])
    ip = lan_ip()
    target = ("127.0.0.1", port)
    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    try:
        server.bind((ip, port))
    except OSError as err:
        sys.exit(f"bind {ip}:{port}: {err}")
    server.listen(128)
    print(f"→ other machines: http://{ip}:{port}", flush=True)
    print(f"→ forwarding to http://127.0.0.1:{port}", flush=True)
    print(f"→ advertise that address: make dev PUBLIC_HOST={ip}", flush=True)
    print("→ Ctrl-C stops the forward", flush=True)
    while True:
        client, _addr = server.accept()
        threading.Thread(target=handle, args=(client, target), daemon=True).start()


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print()
