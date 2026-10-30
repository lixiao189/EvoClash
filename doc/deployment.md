# 部署

Chart 的设计（StatefulSet 身份、扩缩、健康检查）见 [architecture.md](architecture.md#kubernetes)。这里只记怎么装、怎么配。

## Helm

```bash
# netcode 密钥首次安装时随机生成，升级时沿用；也可用 netcodeKey.existingSecret 指定
helm install evoclash oci://ghcr.io/lixiao189/charts/evoclash -n evoclash --create-namespace \
  --set matchmaker.ingress.enabled=true \
  --set 'matchmaker.ingress.hosts[0].host=play.example.com' \
  --set 'matchmaker.ingress.tls[0].hosts[0]=play.example.com' \
  --set 'matchmaker.ingress.tls[0].secretName=play-tls'
```

然后打开 `https://play.example.com/play/`。改过 chart 还没发版时，把 OCI 地址换成仓库里的 `deploy/helm/evoclash`。只想预览生成的清单：

```bash
helm template evoclash deploy/helm/evoclash -n evoclash --set server.autoscaler.bufferSize=5
```

常用参数在 `deploy/helm/evoclash/values.yaml`：镜像标签（默认等于 chart 的 `appVersion`）、匹配服务副本与 HPA、预热池大小、资源配额、节点调度等。

## 容量与恢复

- **房间上限**：`server.maxRooms`（默认 300）。一个对局 Pod 一个房间，扩缩夹在 `server.autoscaler.minReplicas..server.maxRooms`；`replicas` 或 `minReplicas` 超过它时 chart 直接报错。到上限后快速开始去旁观，没有可旁观的就返回 503。单机模式的上限是 `single.maxMatches`。
- **崩溃恢复**：每个对局 Pod 把房间日志写在自己的 PVC 上（`server.journal.persistence`，默认 1Gi、集群默认 StorageClass，缩容时自动删除）。容器崩溃或 Pod 重建后，从卷上恢复原局：match id、种子、端口不变，匹配服务把它标成「恢复中」最多等 90 秒，玩家重连回原局；同一个房间崩溃第 3 次就开新房间。卷是 RWO 时，Pod 换节点要等卷跟过去。关掉持久化退回 emptyDir，只覆盖容器重启。

## 网络

- **对局端口**：节点要放通从 `server.basePort` 起、长度等于 `server.maxRooms` 的 UDP 端口段（默认 7000–7299，`NOTES.txt` 会打印实际范围）。
- **公网地址**：对局默认把 `status.hostIP` 写进令牌；节点外网 IP 与它不同（NAT、云负载均衡）时设 `server.publicHost`。
- **入口**：`LoadBalancer` 会暴露匹配服务的全部接口（包括心跳和 drain）。公网建议改用 `matchmaker.ingress`，它只放行网页（`/`、`/play/`）、`/v1/join`、`/v1/rejoin` 和 `/healthz`。
- **多区域**：每个区域部署一套，各用一个域名，或在前面用 GeoDNS 分流。
- **监控**：匹配服务的 `/metrics` 是 Prometheus 格式。

## Docker

```bash
docker run --network host ghcr.io/lixiao189/evoclash-server --listen 0.0.0.0:8080 --public-host <公网 IP>
```

镜像里带网页客户端；非本机访问需要在前面加一层 HTTPS 反向代理。

## 走向生产还缺什么

- **共享席位预留**：多副本匹配服务的席位预留各自在内存里，靠服务器人数上限兜底。需要严格不超卖时可以把 `Registry` 放进 Redis。
- **全局入口**：可以再加一层返回区域列表、由网页测速选区的服务（`JoinRequest.region` 已预留）。
- **账号与反作弊**：`/v1/join` 前面接入登录；服务器已是权威判定，但还缺速率和异常检测。
- **告警**：`/metrics` 已导出人数、席位、过载对局和 tick 耗时，还缺 Prometheus 规则和看板。
