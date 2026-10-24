# EvoClash

[![CI](https://github.com/lixiao189/EvoClash/actions/workflows/ci.yml/badge.svg)](https://github.com/lixiao189/EvoClash/actions/workflows/ci.yml)

末日废土背景的快节奏联机对战（俯视角 2D）。每个人都拿着一根木棍出生，搜刮前文明的遗物一路升级到动力装甲和轨道炮；死亡后装备全部掉落，回到石器时代。

技术栈：Rust · Bevy 0.20（客户端编译成 wasm，在浏览器里运行）· lightyear 0.31（WebTransport + netcode.io）· axum（匹配服务）

## 快速开始

需要 [trunk](https://trunkrs.dev)（`cargo install trunk`）；`make` 会自动添加 `wasm32-unknown-unknown` target。

```bash
make run
```

然后用 Chrome、Edge 或 Firefox 打开 <http://127.0.0.1:8080>，输入名字即可开玩。匹配服务同时提供网页客户端（`/play/`）和匹配 API，并按需拉起对局进程（带 4 个机器人）。多开几个标签页就是多个玩家。

- `make watch`：另开一个终端，改客户端代码后自动重新构建，刷新页面即可。
- `make run BOTS=0 LISTEN=127.0.0.1:9000`：调整机器人数量和端口。
- `make check`：本地跑一遍 CI 的 fmt / clippy（含 wasm）/ test。

浏览器只允许 HTTPS 页面（`localhost` / `127.0.0.1` 除外）使用 WebTransport，所以局域网或公网上的玩家要通过 HTTPS 访问（见下文 Kubernetes 部署的 ingress）。

操作：WASD 移动 · 鼠标瞄准、左键射击 · 空格冲刺 · E 拾取或替换 · Q/右键 投掷电磁脉冲雷

## 玩法

| 层级 | 武器 | 护甲 / 道具 | 刷新区域 |
|---|---|---|---|
| T0 原始 | 木棍（近战，无限） | — | 出生自带 |
| T1 工业残骸 | 猎弓、管式霰弹枪 | 废铁甲、医疗包、电磁脉冲雷 | 外圈 |
| T2 现代军用 | 制式步枪 | 防弹衣 | 城镇环 |
| T3 信息时代 | 脉冲激光（护甲伤害 ×2） | — | 地堡周围 |
| T4 前文明尖端 | 轨道炮（穿墙、穿人） | 动力装甲（180 点，减速） | 地堡定时开启 |

- **对局**：4 分钟一局，先拿到 15 分者获胜；2–6 人，人数不足时由机器人补位，真人加入后机器人自动让位。
- **成长**：踩到更高层级的装备会自动拾取；弹药打空后退回木棍。
- **防滚雪球**：
  - 死亡掉落全部装备，击杀强者就能继承他的装备。
  - 连杀 3 次会被悬赏（全场可见，击杀得 3 分）。
  - 电磁脉冲雷能清空动力装甲、减速目标，并让能量武器损失一半弹药。
  - 动力装甲会降低移动速度。
  - **落后补给**：复活时按与领先者的分差发装备，落后 2 分得猎弓或霰弹枪，5 分得步枪加废铁甲，8 分得步枪加防弹衣。
  - **以弱打强**：目标装备每比你的武器高一级，你造成的伤害就 +20%（最多 +80%）；用低两级以上的武器击杀对手，额外 +1 分。
  - **复活保护**：复活后 2.5 秒内无敌，开火或投雷会提前结束；机器人不会打处于保护中的玩家。
- **野怪**：废土上游荡着无差别攻击的怪物，不加分但会掉装备，机器人也会优先去打它们（`--no-monsters` 关闭）。
  - 辐射蟑螂：到处乱爬，看见人就直冲过来撕咬。
  - 孢子囊：扎根不动，有人靠近就向四周螺旋喷射慢速孢子弹幕。
- **机器人**：有约 0.3 秒反应时间；多个机器人不会集火同一目标，并且更倾向于打野怪和彼此，而不是真人玩家。
- **地堡事件**：开局 40 秒后，以及之后每 60 秒，地图中心的地堡开启并掉落 T4 装备，所有人都会收到广播。

## 架构

```
        网页 + HTTPS/JSON                 WebTransport / QUIC（netcode.io，加密 + 认证）
 浏览器 ───────────────▶ 匹配服务 ──签发令牌──▶ 浏览器 ═══════════════▶ 对局进程（一局一个）
                          ▲   │ 拉起/回收                                      │
                          │   └──────────────▶ evoclash-server --single-match │
                          └──────────── 每 2 秒心跳（人数、tick 耗时）◀─────────┘
```

| crate | 职责 |
|---|---|
| `crates/game` | 协议、地图、物品表，以及客户端预测和服务器共用的确定性移动规则 |
| `crates/server` | 无渲染的权威服务器：战斗、延迟补偿、机器人、断线保留、心跳 |
| `crates/client` | 浏览器客户端（wasm）：输入、本地预测、渲染、特效、HUD、自动重连 |
| `crates/matchmaker` | 匹配、席位预留、签发连接令牌、进程托管、健康检查，并托管网页客户端 |

### 高并发：横向扩展，而不是一个大进程

- **一局一个进程**，单进程只负责 2–6 人。在 debug 构建下，一局的 tick 平均耗时约 0.3 ms（`/v1/matches` 中的 `tick_us`），只占 15.6 ms tick 预算的约 2%，所以单机可以按核数跑很多局。
- 容量 = 进程数。匹配服务优先把人塞进最满的局，满员或在排空（drain）的局不再分配，席位预留可防止并发加入时超卖。
- 对局之间不共享状态，所以没有锁竞争，也没有跨局的故障传播。
- **单进程只用 1 个工作线程**（`--threads`，默认 1）：一局一 tick 不到 1 ms，多线程只会让同机的几十个对局进程互相抢核。
- **预热池**：匹配服务常备 `--warm-matches`（默认 1）个空局，突发加入不用等进程启动；Kubernetes 上由 Agones FleetAutoscaler 的 buffer 负责。
- **过载避让**：tick 平均耗时超过约 9.4 ms（预算的 60%）的对局说明所在机器 CPU 不够，不再往里分配新玩家。

### 高可用：任何一个组件挂掉，影响都被限制在最小范围

- **对局进程崩溃**：只影响这一局。匹配服务在 6 秒内发现心跳中断并把它摘除，新玩家会被分配到其他局。
- **匹配服务重启**：正在进行的对局不受影响（对局进程不是匹配服务的子进程生命周期绑定，`kill_on_drop(false)`）。新实例通过心跳在 2 秒内自动重新接管全部对局（已实测）。重连密钥是 HMAC，不需要会话存储。
- **玩家断线**：服务器保留其角色 20 秒（装备和分数都在）。客户端带退避自动重连，用重连密钥向匹配服务换新令牌后，接回原来的角色（已实测：挂起客户端 8 秒后恢复）。
- **多副本匹配服务**：`--peer` 列出其他副本，拉起的对局会同时向所有副本发心跳；服务器的 `--matchmaker` 填主机名时会解析出全部地址逐一发心跳（Kubernetes 的 headless Service）。每个副本都有完整的对局视图，任何一个都能分配玩家。各副本的席位预留互不共享，偶尔超卖时由服务器的 netcode 人数上限兜底，被拒的客户端会自动换一局。
- **客户端故障转移**：浏览器从哪个地址加载页面，就向哪个地址请求匹配；多个副本放在同一个负载均衡 / ingress 后面，重连时任一副本都能换新令牌。原对局已经不在（崩溃或结束）时自动加入新对局。
- **发布/维护**：`POST /v1/matches/{id}/drain`，或者给对局进程发 SIGTERM（再发一次立即退出），都会让该局打完当前对局后退出，不会中途踢人；没有真人的对局直接退出。匹配服务也响应 SIGTERM 平滑停机。
- **令牌安全**：连接令牌由匹配服务用共享密钥签名，30 秒过期；玩家名字写在加密的 user data 里，无法伪造。生产环境务必设置 `EVOCLASH_NETCODE_KEY`（64 位十六进制），否则会使用开发密钥。

### 低延迟

- **协议**：WebTransport 的不可靠数据报（QUIC，底层是 UDP，没有 TCP 的队头阻塞），输入每 tick 冗余发送（丢包时不会丢操作）。对局进程启动时生成 14 天有效的自签名证书，证书哈希随心跳报给匹配服务、再随令牌发给浏览器，所以不需要域名证书；进程运行满 12 天会自动排空退出，换新证书。事件分两条通道：击杀和公告走可靠有序，命中和爆炸走不可靠通道。
- **模拟频率**：64 Hz 服务器 tick；状态每 2 tick（约 31 ms）复制一次。
- **客户端预测**：本地移动和冲刺零延迟，误差通过回滚和平滑修正。本地输入不加延迟；预测的位置在固定步之间做插值，高刷新率下也平滑。
- **零延迟射击手感**：自己的弹道在点击瞬间就在本地绘制，服务器不会把你自己的子弹再发回给你；命中由服务器裁决。
- **延迟补偿**：服务器按每个客户端上报的插值延迟，把目标位置回溯到"你屏幕上看到它的那一刻"再判定命中（上限约 220 ms，防止高延迟玩家打"过去"）。
- **调试**：页面地址加 `&lag=80` 可以在客户端模拟网络延迟。

## Kubernetes 部署

Helm chart `deploy/helm/evoclash` 基于 [Agones](https://agones.dev)，用 Kubernetes 接管进程托管、故障恢复和扩缩容：

| 组件 | 资源 | 高可用要点 |
|---|---|---|
| 匹配服务 | `Deployment`（3 副本，HPA 3–10）+ `LoadBalancer` Service | 跨节点/可用区打散，PDB 至少保留 2 个，滚动更新不停服；`--no-launch` 只通过心跳接管对局 |
| 心跳入口 | headless Service `evoclash-matchmaker-heartbeat` | DNS 返回所有匹配服务 Pod，对局向每一个都发心跳 |
| 对局 | Agones `Fleet`（一局一个 Pod，动态 UDP 主机端口，跑 WebTransport） | 卡死的对局不再上报健康，被自动替换；SIGTERM 时打完当前局再退出 |
| 预热池 | Agones `FleetAutoscaler`（Buffer 3） | 常备空闲 Ready 对局，加入不用等 Pod 调度 |

对局进程带 `--agones` 运行时：从 SDK sidecar 读取 GameServer 名字和公网 `节点 IP:主机端口` 作为对局 id 和令牌地址 → 就绪后标记 `Ready` → 只要模拟还在 tick 就上报 `Health` → 第一个真人加入时自己 `Allocate`（Agones 不会缩容进行中的对局）→ 对局结束后 `Shutdown`，Fleet 补一个新的。

```bash
# 1. 安装 Agones，并允许它管理 evoclash 命名空间的 GameServer
helm install agones agones/agones -n agones-system --create-namespace \
  --set "gameservers.namespaces={evoclash}"
# 2. 部署（netcode 密钥首次安装时随机生成，升级时沿用；也可用 netcodeKey.existingSecret 指定）
helm install evoclash oci://ghcr.io/lixiao189/charts/evoclash --version 0.1.0 -n evoclash --create-namespace
#   （改过 chart 还没发版时，用仓库里的目录：helm install evoclash deploy/helm/evoclash ...）
#   浏览器要求 HTTPS 页面才能用 WebTransport，所以给玩家开启带 TLS 的 ingress：
#   --set matchmaker.ingress.enabled=true --set 'matchmaker.ingress.hosts[0].host=play.example.com' \
#   --set 'matchmaker.ingress.tls[0].hosts[0]=play.example.com' --set 'matchmaker.ingress.tls[0].secretName=play-tls'
# 3. 用浏览器打开 https://play.example.com/play/
```

常用参数见 `deploy/helm/evoclash/values.yaml`：镜像标签（默认等于 chart 的 `appVersion`）、匹配服务副本与 HPA、每局人数/机器人、预热池大小、资源配额、节点调度等。只想预览生成的 Pod 清单：

```bash
helm template evoclash deploy/helm/evoclash -n evoclash --set fleet.autoscaler.bufferSize=5
```

节点需要放通 Agones 的 UDP 端口段（默认 7000–8000）。`LoadBalancer` 会把匹配服务的全部接口（包括心跳和 drain）暴露出去；公网部署建议改用 `matchmaker.ingress`，它只放行网页（`/`、`/play/`）、`/v1/join`、`/v1/rejoin` 和 `/healthz`。多区域时每个区域部署一套，各用一个域名，或在前面用 GeoDNS 分流。匹配服务的 `/metrics`（Prometheus 格式）可接入监控。

## 走向生产还缺什么

当前版本是可运行的完整骨架，以下部分是有意留空的接口：

- **共享席位预留**：多副本匹配服务的席位预留各自在内存里，靠服务器人数上限兜底。需要严格不超卖时可以把 `Registry` 放进 Redis。
- **全局入口**：多区域目前靠不同域名或 GeoDNS，可以再加一层返回区域列表、由网页测速选区的服务（`JoinRequest.region` 已预留）。
- **账号与反作弊**：`/v1/join` 前面接入登录；服务器已经是权威判定，但还缺速率和异常检测。
- **告警**：`/metrics` 已导出人数、席位、过载对局和 tick 耗时，还缺 Prometheus 规则和看板。

## 测试

```bash
cargo test --workspace
```

覆盖地图碰撞和寻路、令牌往返、重连密钥校验、席位预留、负载分配、排空、过载避让、预热池计数、Agones GameServer 解析和心跳地址展开。

CI 的镜像冒烟测试会确认匹配服务能提供网页客户端，并拉起一局发回心跳。

## CI / CD

- **CI**（`.github/workflows/ci.yml`）：每次 push 到 `main` 和每个 PR 运行 rustfmt、clippy（`-D warnings`）、三平台 `cargo test`、cargo-deny（漏洞 / 许可证 / 来源）、`helm lint`，并构建服务端 Docker 镜像（不推送）后实际运行：匹配服务拉起一局，等它发回心跳才算通过。这样 `Dockerfile` 写坏了，或者二进制在 distroless 基础镜像里起不来，PR 阶段就能发现。分支保护只需要勾选 `CI success` 这一项。
- **发布**（`.github/workflows/release.yml`）：先把 `Cargo.toml` 的 `workspace.package.version` 和 `deploy/helm/evoclash/Chart.yaml` 的 `version` / `appVersion` 改成新版本并合入，然后推送同名 tag：

  ```bash
  git tag v0.2.0 && git push origin v0.2.0
  ```

  会构建 Linux x86_64/arm64、macOS arm64、Windows x86_64 的服务端二进制（每个压缩包里都带 `web/` 网页客户端，匹配服务默认就会托管它），附 `SHA256SUMS` 和构建来源证明发布到 GitHub Releases，把服务端镜像推到 `ghcr.io/lixiao189/evoclash-server`（amd64 + arm64），镜像推完后再把 Helm chart 作为 OCI 包推到 `oci://ghcr.io/lixiao189/charts/evoclash`，同样附构建来源证明。带 `-` 的 tag（如 `v0.2.0-rc.1`）标记为预发布，不更新 `latest`。

  ```bash
  docker run --network host ghcr.io/lixiao189/evoclash-server --listen 0.0.0.0:8080 --public-host <公网 IP>
  ```

  镜像里已带网页客户端；非本机访问需要在前面加一层 HTTPS 反向代理。

- **依赖更新**：Dependabot 每周提 Cargo 和 Actions 的升级 PR；Bevy / lightyear 单独提，需要手动迁移。
