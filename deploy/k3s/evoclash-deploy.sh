#!/bin/bash
# Forced SSH command (authorized_keys: restrict,command="/usr/local/bin/evoclash-deploy").
# The client runs `deploy <tag>` and pipes a gzip tar on stdin:
#   image.tar   docker save of evoclash-server:<tag>
#   chart/      deploy/helm/evoclash
# containerd names an unqualified docker-save tag docker.io/library/<name>:<tag>
# (reference.ParseNormalizedNamed). Helm's image.repository must be that, or
# pullPolicy Never never finds the image.
set -euo pipefail

export PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
export KUBECONFIG=/etc/rancher/k3s/k3s.yaml

echo "evoclash-deploy: checking command"
cmd=${SSH_ORIGINAL_COMMAND:-}
tag=${cmd#deploy }
if [[ "$cmd" != "deploy ${tag}" || ! "$tag" =~ ^[0-9a-f]{7,40}$ ]]; then
  echo "evoclash-deploy: usage: deploy <tag>  (tag must match ^[0-9a-f]{7,40}$)" >&2
  exit 1
fi

if [[ ! -r "$KUBECONFIG" ]]; then
  echo "evoclash-deploy: missing $KUBECONFIG" >&2
  exit 1
fi
if [[ ! -r /etc/evoclash/values.yaml ]]; then
  echo "evoclash-deploy: missing /etc/evoclash/values.yaml" >&2
  exit 1
fi

umask 077
workdir=$(mktemp -d)
trap 'rm -rf "$workdir"' EXIT

echo "evoclash-deploy: reading bundle"
cat > "$workdir/bundle.tgz"
# Reject absolute paths and `..` before extracting as root.
unsafe=$(tar -tzf "$workdir/bundle.tgz" | grep -E '(^/|(^|/)\.\.(/|$))' || true)
if [[ -n "$unsafe" ]]; then
  echo "evoclash-deploy: refusing unsafe tar paths:" >&2
  printf '%s\n' "$unsafe" >&2
  exit 1
fi
listing=$(tar -tzf "$workdir/bundle.tgz")
# GNU tar may prefix `./`. Either form is the bundle we pack in deploy.yml.
if ! grep -Eq '^(\./)?image\.tar$' <<<"$listing"; then
  echo "evoclash-deploy: bundle missing image.tar" >&2
  exit 1
fi
if ! grep -Eq '^(\./)?chart/Chart\.yaml$' <<<"$listing"; then
  echo "evoclash-deploy: bundle missing chart/Chart.yaml" >&2
  exit 1
fi

echo "evoclash-deploy: extracting"
mkdir -p "$workdir/src"
tar -xzf "$workdir/bundle.tgz" -C "$workdir/src"
src=$workdir/src

ref="docker.io/library/evoclash-server:${tag}"
echo "evoclash-deploy: importing image.tar (expect ${ref})"
k3s ctr -n k8s.io images import "$src/image.tar"
names=$(k3s ctr -n k8s.io images ls -q)
if ! grep -Fxq "$ref" <<<"$names"; then
  echo "evoclash-deploy: ctr did not import ${ref}" >&2
  echo "evoclash-deploy: names containing evoclash-server:" >&2
  grep evoclash-server <<<"$names" >&2 || true
  exit 1
fi

# Tag currently running, so a bad upgrade can still roll back to it. Captured before helm.
prev=""
if prev_image=$(k3s kubectl -n evoclash get deploy -l app.kubernetes.io/name=evoclash,app.kubernetes.io/component=single -o jsonpath='{.items[0].spec.template.spec.containers[?(@.name=="matchmaker")].image}' 2>/dev/null); then
  if [[ -n "$prev_image" ]]; then
    prev=${prev_image##*:}
    echo "evoclash-deploy: previous image ${prev_image}"
  fi
fi
if [[ -z "$prev" ]]; then
  echo "evoclash-deploy: no previous deployment"
fi

echo "evoclash-deploy: helm upgrade evoclash tag ${tag}"
helm upgrade --install evoclash "$src/chart" -n evoclash --create-namespace \
  -f /etc/evoclash/values.yaml \
  --set image.repository=docker.io/library/evoclash-server \
  --set "image.tag=${tag}" \
  --wait --timeout 5m

echo "evoclash-deploy: pruning evoclash-server images except ${tag} and ${prev:-<none>}"
while IFS= read -r img; do
  [[ -n "$img" ]] || continue
  case "$img" in
    docker.io/library/evoclash-server:*)
      img_tag=${img##*:}
      if [[ "$img_tag" != "$tag" && "$img_tag" != "$prev" ]]; then
        echo "evoclash-deploy: removing ${img}"
        k3s ctr -n k8s.io images rm "$img" || echo "evoclash-deploy: could not remove ${img}" >&2
      fi
      ;;
  esac
done < <(k3s ctr -n k8s.io images ls -q)

echo "evoclash-deploy: done (${tag})"
