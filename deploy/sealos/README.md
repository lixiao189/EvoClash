# Sealos（广州 gzg.sealos.run）

一个 Sealos 命名空间（默认 `ns-80h1uphq`）。只用单机模式（`single.enabled`），PodSecurity 是 baseline，**不允许 hostNetwork**；配额里 NodePort 只有 10 个。图表开 `single.enabled` + `single.hostNetwork: false`，只渲染 netcode Secret、一个 Deployment（matchmaker 拉起对局子进程）、ClusterIP Service、UDP NodePort Service 和 Ingress。

## 流量

- 浏览器：`https://evoclash-80h1uphq.gzg.sealos.run/play/`。Sealos 网关（Envoy）终止 TLS，用平台的 `*.gzg.sealos.run` Let's Encrypt 通配证书。Ingress 写 `kubernetes.io/ingress.class: nginx`，**必须带 `tls` 段**（`secretName: wildcard-cert`，本命名空间里没有这个 Secret，网关回落到平台证书），否则该域名只有 HTTP 能路由，HTTPS 是 404。只暴露 `/`、`/play`、`/v1/join`、`/v1/rooms`、`/v1/rejoin`、`/healthz`；心跳、drain、metrics 不出集群。
- 对局：WebTransport（QUIC/UDP）直连 `single.publicHost` 的 NodePort `31700-31707`。nodePort == 容器端口 == matchmaker 下发的端口，`--ports` 照常用。所有节点公网 IP 都转发这些 NodePort（`externalTrafficPolicy: Cluster`），`publicHost` 取 `gzg.sealos.run` 的一个 A 记录（`dig +short @1.1.1.1 gzg.sealos.run`）。
- 限流 IP：网关从 `172.16.0.0/12` 回源，并把 `X-Real-IP` 改写成玩家 IP。matchmaker 的 `--trusted-proxy 172.16.0.0/12` 只采信这些来源的 `X-Real-IP`。

## 一次性准备

1. Sealos 控制台下载 kubeconfig（只给本命名空间权限）。不要提交进仓库。
2. 首次 CD 推送后，把 GitHub 包 `evoclash-server` 设成 **Public**（Packages → Package settings → Change visibility），否则集群拉不到镜像。若要保持私有：在命名空间里建 `kubectl create secret docker-registry ghcr-pull --docker-server=ghcr.io --docker-username=<user> --docker-password=<read:packages PAT>`，并在 `values-sealos.yaml` 加 `imagePullSecrets: [{ name: ghcr-pull }]`。
3. 确认 NodePort `31700-31707` 没被别的租户占用（`helm upgrade` 报 `provided port is already allocated` 时换一段，改 `single.ports`，最多 10 个）。

## GitHub

Environment 名 `sealos`。`main` 上 CI 通过后，Deploy 的 `sealos` 任务和 k3s 任务并行：下载 CI 冒烟测试过的镜像（artifact `server-image`），推到 `ghcr.io/<owner>/evoclash-server:<tag>`，再 `helm upgrade --install evoclash … -f deploy/sealos/values-sealos.yaml --set image.tag=<tag> --wait`。标签不变时 Pod 不重建。

| 名字 | 种类 | 内容 |
|---|---|---|
| `SEALOS_KUBECONFIG` | secret | kubeconfig 全文 |
| `SEALOS_NAMESPACE` | variable | 可选，默认 `ns-80h1uphq` |
| `SEALOS_PUBLIC_HOST` | variable | 可选，覆盖 `single.publicHost`（IP，不是域名） |
| `SEALOS_HOST` | variable | 可选，覆盖 Ingress 域名（须在 `*.gzg.sealos.run` 下） |

## 手动部署

```bash
export KUBECONFIG=~/Downloads/kubeconfig.yaml
helm upgrade --install evoclash deploy/helm/evoclash -n ns-80h1uphq \
  -f deploy/sealos/values-sealos.yaml --set image.tag=<tag> --wait
```

## 销毁

```bash
helm uninstall evoclash -n ns-80h1uphq
# netcode Secret 带 helm.sh/resource-policy: keep，卸载后保留；不要了再删：
kubectl -n ns-80h1uphq delete secret evoclash-netcode
```

## 注意

- 对局子进程和 matchmaker 在同一个容器里，发版（Recreate）会打断正在进行的对局。
- LimitRange 给没写 limits 的容器默认 50m CPU / 64Mi，`values-sealos.yaml` 里的 `resources` 不能删。
- `publicHost` 是固定 IP；平台换节点 IP 后要改 `SEALOS_PUBLIC_HOST`。
