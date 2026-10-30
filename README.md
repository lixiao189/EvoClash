# EvoClash

[![CI](https://github.com/lixiao189/EvoClash/actions/workflows/ci.yml/badge.svg)](https://github.com/lixiao189/EvoClash/actions/workflows/ci.yml)

末日废土背景的快节奏联机对战（俯视角 2D）。每个人都拿着一根木棍出生，搜刮前文明的遗物一路升级到动力装甲和轨道炮；死亡后装备全部掉落，回到石器时代。

技术栈：Rust · Bevy 0.20（客户端编译成 wasm，在浏览器里运行）· lightyear 0.31（WebTransport + netcode.io）· axum（匹配服务）

## 快速开始

需要 [trunk](https://trunkrs.dev)（`cargo install trunk`）；`make` 会自动添加 `wasm32-unknown-unknown` target。

```bash
make run
```

然后用 Chrome、Edge 或 Firefox 打开 <http://127.0.0.1:8080>，输入名字即可开玩。匹配服务同时提供网页客户端（`/play/`）和匹配 API，并按需拉起对局进程；每局固定 8 名战斗者，机器人补位、真人加入时让位。多开几个标签页就是多个玩家。

- `make watch`：另开一个终端，改客户端代码后自动重新构建，刷新页面即可。
- `make dev` / `make watch-dev`：同上，但客户端用调试版构建（改代码后几秒就能重建，浏览器里运行较慢），日常调试用。
- `make run LISTEN=127.0.0.1:9000`：换个端口（每局固定 8 人，机器人由服务器补满）。
- `make check`：本地跑一遍 CI 的 fmt / clippy（含 wasm）/ test。

浏览器只允许 HTTPS 页面（`localhost` / `127.0.0.1` 除外）使用 WebTransport，所以局域网或公网上的玩家要通过 HTTPS 访问（见下文 Kubernetes 部署的 ingress）。

操作：WASD 移动 · 鼠标瞄准、左键射击 · 空格冲刺 · E 拾取或替换（按住撬保险柜） · Q/右键 投掷电磁脉冲雷 · R/T 技能 · Tab 进化面板

## 玩法

木棍出生，搜刮 T1–T4 的枪和护甲，死亡掉落全部装备。8 分钟一局，每局固定 8 名战斗者（真人优先、机器人补位）；没有先到分即胜的规则，打满 8 分钟后按最终分数排名决出胜者，同分先比击杀数、再比死亡数、再比谁先达到最终分数。打完 15 秒结算后自动进入下一局，一直循环。公开房第一局有 20 秒大厅、之后每局 15 秒，私人房用 4 位房间码。规则、数值和房间流程在 [doc/gameplay.md](doc/gameplay.md)。

## 架构

浏览器向匹配服务要令牌，再用 WebTransport 连到单独的对局进程。一个房间一个进程（一局接一局），64 Hz 权威模拟，客户端预测自己的移动。高并发、高可用、延迟、日志、重放和崩溃恢复的说明在 [doc/architecture.md](doc/architecture.md)。

## Kubernetes 部署

Helm chart `deploy/helm/evoclash` 只用 Kubernetes 原生对象（Deployment、StatefulSet、Lease、scale 子资源）接管进程托管和扩缩容：

| 组件 | 资源 | 高可用要点 |
|---|---|---|
| 匹配服务 | `Deployment`（3 副本，HPA 3–10）+ `LoadBalancer` Service | 跨节点/可用区打散，PDB 至少保留 2 个，滚动更新不停服；`--no-launch` 只通过心跳接管对局 |
| 心跳入口 | headless Service `evoclash-matchmaker-heartbeat` | DNS 返回所有匹配服务 Pod，对局向每一个都发心跳 |
| 对局 | `StatefulSet`（一个 Pod 一个房间，`hostNetwork`，UDP 端口 = `basePort + ordinal`） | 卡死的进程被 livenessProbe 重启；SIGTERM 时打完当前局再退出；房间打完自动回大厅开下一局，空房间就是热池 |
| 预热池 | 匹配服务自己扩缩 `statefulsets/scale`（Buffer 3） | 常备空闲空局，加入不用等 Pod 调度；缩容只砍空的、最高 ordinal 的 Pod |

```bash
# 部署（netcode 密钥首次安装时随机生成，升级时沿用；也可用 netcodeKey.existingSecret 指定）
helm install evoclash oci://ghcr.io/lixiao189/charts/evoclash --version 0.1.0 -n evoclash --create-namespace
#   （改过 chart 还没发版时，用仓库里的目录：helm install evoclash deploy/helm/evoclash ...）
#   浏览器要求 HTTPS 页面才能用 WebTransport，所以给玩家开启带 TLS 的 ingress：
#   --set matchmaker.ingress.enabled=true --set 'matchmaker.ingress.hosts[0].host=play.example.com' \
#   --set 'matchmaker.ingress.tls[0].hosts[0]=play.example.com' --set 'matchmaker.ingress.tls[0].secretName=play-tls'
# 用浏览器打开 https://play.example.com/play/
```

对局 Pod 是 `<release>-server-N`，N 就是 ordinal：它绑定宿主机的 UDP `server.basePort + N`（`hostNetwork`，同节点多个房间端口不冲突），公网地址默认取 downward API 的 `status.hostIP`。节点外网 IP 和 hostIP 不同（NAT、云负载均衡）时设 `server.publicHost`，所有对局都用它写令牌。`--statefulset` 从 `HOSTNAME` 推出 ordinal、端口和地址，房间像本地 fleet 一样一局接一局地循环（进程不退出）；match_id 是 `<pod>-<启动后缀>`，只在崩溃重启后变，新进程不会继承上一份进程在注册表里的记录。

扩缩由匹配服务做：用 ServiceAccount 调 API server，Lease 选主，把 `statefulsets/<name>/scale` 改成「有人的局数 + `server.autoscaler.bufferSize`」，夹在 `minReplicas..maxReplicas`。快速开始优先把玩家放进 ordinal 小的房间（按数字比较，`server-2` 在 `server-10` 前面），让高 ordinal 先空出来；缩容前先把要砍的 ordinal 在注册表里标成不分配新玩家，只砍空的最高 ordinal，最高 ordinal 有人在打就等这一局结束。

常用参数见 `deploy/helm/evoclash/values.yaml`：镜像标签（默认等于 chart 的 `appVersion`）、匹配服务副本与 HPA、每局人数与机器人（固定 8 人）、预热池大小、资源配额、节点调度等。只想预览生成的 Pod 清单：

```bash
helm template evoclash deploy/helm/evoclash -n evoclash --set server.autoscaler.bufferSize=5
```

节点需要放通 `server.basePort` 起、长度等于副本上限的 UDP 端口段（默认 7000–7299），这是对局 WebTransport 用的。`LoadBalancer` 会把匹配服务的全部接口（包括心跳和 drain）暴露出去；公网部署建议改用 `matchmaker.ingress`，它只放行网页（`/`、`/play/`）、`/v1/join`、`/v1/rejoin` 和 `/healthz`。多区域时每个区域部署一套，各用一个域名，或在前面用 GeoDNS 分流。匹配服务的 `/metrics`（Prometheus 格式）可接入监控。

## 走向生产还缺什么

崩溃恢复只在本地 `fleet` 和单机部署上生效，Kubernetes 上没有。下面这些接口仍然是空的：

- **共享席位预留**：多副本匹配服务的席位预留各自在内存里，靠服务器人数上限兜底。需要严格不超卖时可以把 `Registry` 放进 Redis。
- **全局入口**：多区域目前靠不同域名或 GeoDNS，可以再加一层返回区域列表、由网页测速选区的服务（`JoinRequest.region` 已预留）。
- **账号与反作弊**：`/v1/join` 前面接入登录；服务器已经是权威判定，但还缺速率和异常检测。
- **告警**：`/metrics` 已导出人数、席位、过载对局和 tick 耗时，还缺 Prometheus 规则和看板。

## 测试

```bash
cargo test --workspace
```

覆盖地图碰撞和寻路、令牌往返、重连密钥校验、席位预留、负载分配、排空、过载避让、预热池计数、StatefulSet 身份解析（ordinal / 端口 / match_id / 健康文件）、扩缩决策和心跳地址展开。

CI 的镜像冒烟测试会确认匹配服务能提供网页客户端，并拉起一局发回心跳。

## CI / CD

- **CI**（`.github/workflows/ci.yml`）：每次 push 到 `main` 和每个 PR 运行 rustfmt、clippy（`-D warnings`，含 wasm 客户端）、`cargo test`、cargo-deny（漏洞 / 许可证 / 来源）、gitleaks 敏感信息扫描（`.gitleaks.toml`：默认密钥规则 + 公网 IPv4）、`helm lint`，并构建服务端 Docker 镜像（不推送）后实际运行：匹配服务拉起一局，等它发回心跳才算通过。这样 `Dockerfile` 写坏了，或者二进制在 distroless 基础镜像里起不来，PR 阶段就能发现。分支保护只需要勾选 `CI success` 这一项。
- **发布**（`.github/workflows/release.yml`）：先把 `Cargo.toml` 的 `workspace.package.version` 和 `deploy/helm/evoclash/Chart.yaml` 的 `version` / `appVersion` 改成新版本并合入，然后推送同名 tag：

  ```bash
  git tag v0.2.0 && git push origin v0.2.0
  ```

  会构建 wasm 网页客户端，打包成 `evoclash-web-vX.Y.Z.tar.gz`，附 `SHA256SUMS` 和构建来源证明发布到 GitHub Releases（不再提供各平台的二进制）；再把服务端镜像（对局服务器 + 匹配服务 + 网页客户端，amd64 + arm64）推到 `ghcr.io/lixiao189/evoclash-server`，镜像推完后再把 Helm chart 作为 OCI 包推到 `oci://ghcr.io/lixiao189/charts/evoclash`，同样附构建来源证明。带 `-` 的 tag（如 `v0.2.0-rc.1`）标记为预发布，不更新 `latest`。

  ```bash
  docker run --network host ghcr.io/lixiao189/evoclash-server --listen 0.0.0.0:8080 --public-host <公网 IP>
  ```

  镜像里已带网页客户端；非本机访问需要在前面加一层 HTTPS 反向代理。

- **依赖更新**：Dependabot 每周提 Cargo 和 Actions 的升级 PR；Bevy / lightyear 单独提，需要手动迁移。

## 许可证

代码采用 MIT OR Apache-2.0。美术素材（`crates/client/assets/lpc/`，来自 [Liberated Pixel Cup](https://lpc.opengameart.org/) 系列）是其他作者的作品，**不**适用代码的许可证：默认按 CC-BY-SA 3.0 分发，详见 [`LICENSE-ART.md`](crates/client/assets/LICENSE-ART.md)；每个文件的作者、来源和许可证列在 [`CREDITS.md`](crates/client/assets/CREDITS.md)，游戏内可在 `/play/credits.html` 查看。新增素材文件时必须同时补上致谢条目，否则 `cargo test` 的 `every_art_file_is_credited` 会失败。
