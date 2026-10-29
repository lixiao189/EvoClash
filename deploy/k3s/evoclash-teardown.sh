#!/bin/bash
# Undo the single-node k3s install and the host files added around it.
# Does not touch sshd, sing-box, iperf3, unrelated ufw rules, or other SSH keys.
set -u

DRY=0
YES=0

usage() {
  echo "usage: evoclash-teardown [--yes] [--dry-run]"
}

for arg in "$@"; do
  case "$arg" in
    --yes) YES=1 ;;
    --dry-run) DRY=1 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $arg" >&2; usage >&2; exit 2 ;;
  esac
done

cat <<'EOF'
evoclash-teardown will remove:
  1. Helm release evoclash (namespace evoclash), if k3s is active; errors ignored
  2. k3s via /usr/local/bin/k3s-uninstall.sh (service, containers, images, CNI,
     k3s iptables rules, /var/lib/rancher/k3s, /etc/rancher/k3s, k3s binaries)
  3. /etc/systemd/system/k3s.service.d/gogc.conf (and the dir if empty)
  4. nft table inet evoclash_guard, /etc/nftables.d/evoclash-guard.nft,
     evoclash-guard.service (disabled); /etc/nftables.d if we leave it empty
  5. /usr/local/bin/helm
  6. exact KUBECONFIG lines in /root/.bashrc and /etc/environment
  7. /etc/evoclash/ and /usr/local/bin/evoclash-deploy
  8. authorized_keys lines containing command="/usr/local/bin/evoclash-deploy"
     (backup beside the file first; other keys kept)
  9. ufw allows added for k3s only: tcp 6443 and 10250 from 10.42.0.0/16 and
     10.43.0.0/16, plus route allow from/to 10.42.0.0/16, and every rule
     commented "evoclash" (38443/tcp from Cloudflare, 37000:37063/udp)
 10. net.ipv4.ip_forward reset to 0 (k3s turned it on; no sysctl.d file)
 11. leftovers uninstall misses: /etc/rancher/node, empty /etc/rancher,
     empty /var/lib/rancher
 12. systemctl daemon-reload, then this script (/usr/local/sbin/evoclash-teardown)
left alone: sshd, sing-box, iperf3, /etc/nftables.conf, other ufw rules, other keys
EOF

if [[ "$DRY" -eq 1 ]]; then
  echo "dry-run: no changes made"
  exit 0
fi

if [[ "$(id -u)" -ne 0 ]]; then
  echo "must be root" >&2
  exit 1
fi

if [[ "$YES" -ne 1 ]]; then
  printf 'Type yes to proceed: '
  read -r ans
  if [[ "$ans" != "yes" ]]; then
    echo "aborted"
    exit 1
  fi
fi

strip_exact_line() {
  local file=$1 line=$2 tmp
  [[ -f "$file" ]] || return 0
  grep -qxF "$line" "$file" || return 0
  tmp=$(mktemp "$(dirname "$file")/.teardown.XXXXXX")
  grep -vxF "$line" "$file" >"$tmp" || true
  chmod --reference="$file" "$tmp"
  chown --reference="$file" "$tmp"
  mv "$tmp" "$file"
}

if systemctl is-active --quiet k3s 2>/dev/null; then
  if [[ -x /usr/local/bin/helm ]]; then
    env KUBECONFIG=/etc/rancher/k3s/k3s.yaml \
      helm uninstall evoclash -n evoclash || true
  fi
fi

if [[ -x /usr/local/bin/k3s-uninstall.sh ]]; then
  /usr/local/bin/k3s-uninstall.sh || true
elif [[ -x /usr/local/bin/k3s-killall.sh ]]; then
  /usr/local/bin/k3s-killall.sh || true
fi

rm -f /etc/systemd/system/k3s.service.d/gogc.conf
rmdir /etc/systemd/system/k3s.service.d 2>/dev/null || true

systemctl disable --now evoclash-guard.service 2>/dev/null || true
rm -f /etc/systemd/system/evoclash-guard.service
rm -f /etc/systemd/system/multi-user.target.wants/evoclash-guard.service
nft delete table inet evoclash_guard 2>/dev/null || true
rm -f /etc/nftables.d/evoclash-guard.nft
rmdir /etc/nftables.d 2>/dev/null || true

rm -f /usr/local/bin/helm

strip_exact_line /root/.bashrc 'export KUBECONFIG=/etc/rancher/k3s/k3s.yaml'
strip_exact_line /etc/environment 'KUBECONFIG=/etc/rancher/k3s/k3s.yaml'

rm -rf /etc/evoclash
rm -f /usr/local/bin/evoclash-deploy

ak=/root/.ssh/authorized_keys
needle='command="/usr/local/bin/evoclash-deploy"'
if [[ -f "$ak" ]] && grep -qF "$needle" "$ak"; then
  cp -a "$ak" "${ak}.bak.$(date +%Y%m%d%H%M%S).$$"
  tmp=$(mktemp "$(dirname "$ak")/.authorized_keys.XXXXXX")
  grep -vF "$needle" "$ak" >"$tmp" || true
  chmod --reference="$ak" "$tmp"
  chown --reference="$ak" "$tmp"
  mv "$tmp" "$ak"
fi

if command -v ufw >/dev/null 2>&1; then
  ufw --force delete allow from 10.42.0.0/16 to any port 6443 proto tcp || true
  ufw --force delete allow from 10.43.0.0/16 to any port 6443 proto tcp || true
  ufw --force delete allow from 10.42.0.0/16 to any port 10250 proto tcp || true
  ufw --force delete allow from 10.43.0.0/16 to any port 10250 proto tcp || true
  ufw --force route delete allow from 10.42.0.0/16 || true
  ufw --force route delete allow to 10.42.0.0/16 || true
  # Game ports: 38443/tcp from Cloudflare ranges and the UDP match range, all commented "evoclash".
  while n=$(ufw status numbered | grep -m1 '# evoclash' | sed -n 's/^\[ *\([0-9]*\)\].*/\1/p'); [[ -n "$n" ]]; do
    ufw --force delete "$n" >/dev/null || break
  done
fi

sysctl -w net.ipv4.ip_forward=0 >/dev/null || true

rm -rf /etc/rancher/node
rmdir /etc/rancher 2>/dev/null || true
rmdir /var/lib/rancher 2>/dev/null || true

systemctl daemon-reload || true
rm -f /usr/local/sbin/evoclash-teardown
echo "evoclash-teardown: done"
