# 单机 k3s

一台小机器：k3s 关掉 traefik、servicelb、metrics-server，不装 Agones。`single.enabled` 时图表只渲染 netcode Secret、nginx ConfigMap 和一个 hostNetwork Deployment。

## 流量

浏览器走 Cloudflare 橙云 HTTPS，回源到本机 `:38443`。Pod 里的 nginx 用源站证书，并开启 Authenticated Origin Pulls（校验 Cloudflare 客户端证书，CA 打在 ConfigMap 里），只反代 `/`、`/play`、`/v1/join`、`/v1/rooms`、`/v1/rejoin`、`/healthz` 到 `127.0.0.1:8080` 的 matchmaker。`X-Real-IP` 取 `CF-Connecting-IP`，回环来源才采信。

对局是 WebTransport（QUIC，UDP `37000-37063`），直连 `single.publicHost`。Cloudflare 不能代理 UDP，这几段端口要对玩家放行。

## 一次性准备

```bash
curl -sfL https://get.k3s.io | sh -s - --disable=traefik --disable=servicelb --disable=metrics-server
# helm 3 放到 /usr/local/bin/helm。强制命令的 PATH 很短，k3s 和 helm 都要在 /usr/local/bin 或 /usr/bin。
install -m 755 deploy/k3s/evoclash-deploy.sh /usr/local/bin/evoclash-deploy
install -d -m 755 /etc/evoclash
install -m 644 deploy/k3s/values-k3s.yaml /etc/evoclash/values.yaml
```

`~/.ssh/authorized_keys`（部署公钥）一行：

```text
restrict,command="/usr/local/bin/evoclash-deploy" ssh-ed25519 AAAA... deploy
```

源站证书（Cloudflare Origin Certificate）要在第一次发布之前放进集群。Pod 用 hostNetwork，命名空间不能是 Restricted：

```bash
kubectl create namespace evoclash
kubectl -n evoclash create secret tls evoclash-origin-tls --cert=origin.pem --key=origin.key
```

Cloudflare：DNS 橙云；Origin Rule 把 `game.node189.top` 回源端口改成 `38443`；打开 Authenticated Origin Pulls（区域级，不请求客户端证书的其它源站不受影响）；SSL 至少 Full（同区有别的站点时用只匹配该主机名的 Configuration Rule 设 Strict）。UFW 只对 [Cloudflare IP 段](https://www.cloudflare.com/ips/) 放行 TCP `38443`，对所有人放行 UDP `37000:37063`，规则注释都带 `evoclash`。SSH 监听 `1145`。游戏镜像不走仓库；节点要能拉取 `nginxinc/nginx-unprivileged:1.27-alpine`（或自行 import）。

`docker save evoclash-server:<tag>` 经 `k3s ctr -n k8s.io images import` 后的名字是 `docker.io/library/evoclash-server:<tag>`。部署脚本会 `--set` 这个 repository；`pullPolicy: Never` 写在 values 里。

## GitHub

Environment 名 `production`。`main` 上的 CI 通过后，Deploy（`workflow_run`）不再编译：直接下载 CI `image` 任务冒烟测试过的 release 镜像（artifact `server-image`，标签是二进制、网页和 Dockerfile 的内容哈希前 12 位），连同同一提交的 chart 打成 gzip tar，`ssh … deploy <tag>` 送过去。内容没变时标签不变，helm 不会重建 Pod、打断对局。手动跑 Deploy 部署最近一次通过的 main CI。

| 名字 | 种类 | 内容 |
|---|---|---|
| `DEPLOY_SSH_KEY` | secret | 上面那把私钥（保留换行） |
| `DEPLOY_KNOWN_HOSTS` | secret | `ssh-keyscan -p 1145 179.255.113.5` |
| `DEPLOY_HOST` | variable | 默认 `179.255.113.5` |
| `DEPLOY_PORT` | variable | 默认 `1145` |

## 销毁

服务器上的 `/usr/local/sbin/evoclash-teardown`（仓库里是 `deploy/k3s/evoclash-teardown.sh`）卸载 evoclash 和 k3s，并撤掉上面的全部改动（含 `evoclash` UFW 规则和部署公钥）。先 `--dry-run` 看清单，`--yes` 跳过确认。
