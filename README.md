# EvoClash

[![CI](https://github.com/lixiao189/EvoClash/actions/workflows/ci.yml/badge.svg)](https://github.com/lixiao189/EvoClash/actions/workflows/ci.yml)

末日废土背景的快节奏联机对战（俯视角 2D）。每个人都拿着一根木棍出生，搜刮前文明的遗物一路升级到动力装甲和轨道炮；死亡后装备全部掉落，回到石器时代。

技术栈：Rust · Bevy 0.20 · lightyear 0.31（UDP + netcode.io）· axum（匹配服务）

## 快速开始

```bash
cargo build --release
```

**本地直连（开发用）**：

```bash
./target/release/evoclash-server --port 7000 --bots 4
./target/release/evoclash --server 127.0.0.1:7000 --id 1
```

**走匹配服务（完整链路）**：匹配服务按需拉起对局进程。

```bash
./target/release/evoclash-matchmaker --listen 127.0.0.1:8080
./target/release/evoclash --matchmaker http://127.0.0.1:8080 --name 阿狸
```

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
- **地堡事件**：开局 40 秒后，以及之后每 60 秒，地图中心的地堡开启并掉落 T4 装备，所有人都会收到广播。

## 架构

```
             HTTPS/JSON                         UDP（netcode.io，加密 + 认证）
 客户端 ───────────────▶ 匹配服务 ──签发令牌──▶ 客户端 ═══════════════▶ 对局进程（一局一个）
                          ▲   │ 拉起/回收                                      │
                          │   └──────────────▶ evoclash-server --single-match │
                          └──────────── 每 2 秒心跳（人数、tick 耗时）◀─────────┘
```

| crate | 职责 |
|---|---|
| `crates/game` | 协议、地图、物品表，以及客户端预测和服务器共用的确定性移动规则 |
| `crates/server` | 无渲染的权威服务器：战斗、延迟补偿、机器人、断线保留、心跳 |
| `crates/client` | 输入、本地预测、渲染、特效、HUD、自动重连 |
| `crates/matchmaker` | 匹配、席位预留、签发连接令牌、进程托管、健康检查 |

### 高并发：横向扩展，而不是一个大进程

- **一局一个进程**，单进程只负责 2–6 人。在 debug 构建下，一局的 tick 平均耗时约 0.3 ms（`/v1/matches` 中的 `tick_us`），只占 15.6 ms tick 预算的约 2%，所以单机可以按核数跑很多局。
- 容量 = 进程数。匹配服务优先把人塞进最满的局，满员或在排空（drain）的局不再分配，席位预留可防止并发加入时超卖。
- 对局之间不共享状态，所以没有锁竞争，也没有跨局的故障传播。

### 高可用：任何一个组件挂掉，影响都被限制在最小范围

- **对局进程崩溃**：只影响这一局。匹配服务在 6 秒内发现心跳中断并把它摘除，新玩家会被分配到其他局。
- **匹配服务重启**：正在进行的对局不受影响（对局进程不是匹配服务的子进程生命周期绑定，`kill_on_drop(false)`）。新实例通过心跳在 2 秒内自动重新接管全部对局（已实测）。重连密钥是 HMAC，不需要会话存储。
- **玩家断线**：服务器保留其角色 20 秒（装备和分数都在）。客户端带退避自动重连，用重连密钥向匹配服务换新令牌后，接回原来的角色（已实测：挂起客户端 8 秒后恢复）。
- **发布/维护**：`POST /v1/matches/{id}/drain` 让该局打完当前对局后退出，不会中途踢人。
- **令牌安全**：连接令牌由匹配服务用共享密钥签名，30 秒过期；玩家名字写在加密的 user data 里，无法伪造。生产环境务必设置 `EVOCLASH_NETCODE_KEY`（64 位十六进制），否则会使用开发密钥。

### 低延迟

- **协议**：UDP，输入每 tick 冗余发送（丢包时不会丢操作）。事件分两条通道：击杀和公告走可靠有序，命中和爆炸走不可靠通道。
- **模拟频率**：64 Hz 服务器 tick；状态每 2 tick（约 31 ms）复制一次。
- **客户端预测**：本地移动和冲刺零延迟，误差通过回滚和平滑修正。本地输入不加延迟；预测的位置在固定步之间做插值，高刷新率下也平滑。
- **零延迟射击手感**：自己的弹道在点击瞬间就在本地绘制，服务器不会把你自己的子弹再发回给你；命中由服务器裁决。
- **延迟补偿**：服务器按每个客户端上报的插值延迟，把目标位置回溯到"你屏幕上看到它的那一刻"再判定命中（上限约 220 ms，防止高延迟玩家打"过去"）。
- **调试**：`--fake-latency 80` 可以在客户端模拟网络延迟。

## 走向生产还缺什么

当前版本是可运行的完整骨架，以下部分是有意留空的接口：

- **编排**：把 `crates/matchmaker/src/fleet.rs`（本机拉起进程）替换为 Agones `GameServerAllocation`、Edgegap 或 Hathora 等。匹配服务的 `--no-launch` 模式已经支持只接管外部拉起的服务器。
- **多副本匹配服务**：席位预留目前在单个实例的内存里。部署多副本时需要把 `Registry` 放进 Redis，并让心跳打到负载均衡地址。
- **多区域**：每个区域部署一套匹配服务和对局集群，客户端按延迟选区（`JoinRequest.region` 已预留）。
- **账号与反作弊**：`/v1/join` 前面接入登录；服务器已经是权威判定，但还缺速率和异常检测。
- **指标**：把心跳数据（人数、tick 耗时）导出到 Prometheus 做自动扩缩容。

## 测试

```bash
cargo test --workspace
```

覆盖地图碰撞和寻路、令牌往返、重连密钥校验、席位预留、负载分配和排空。

联机冒烟测试（无窗口、自动操作）：

```bash
evoclash --server 127.0.0.1:7000 --headless --autopilot
```

## CI / CD

- **CI**（`.github/workflows/ci.yml`）：每次 push 到 `main` 和每个 PR 运行 rustfmt、clippy（`-D warnings`）、三平台 `cargo test`、cargo-deny（漏洞 / 许可证 / 来源）。分支保护只需要勾选 `CI success` 这一项。
- **发布**（`.github/workflows/release.yml`）：先把 `Cargo.toml` 的 `workspace.package.version` 改成新版本并合入，然后推送同名 tag：

  ```bash
  git tag v0.2.0 && git push origin v0.2.0
  ```

  会构建 Linux x86_64/arm64、macOS arm64、Windows x86_64 的二进制，附 `SHA256SUMS` 和构建来源证明发布到 GitHub Releases，并把服务端镜像推到 `ghcr.io/lixiao189/evoclash-server`（amd64 + arm64）。带 `-` 的 tag（如 `v0.2.0-rc.1`）标记为预发布，不更新 `latest`。

  ```bash
  docker run --network host ghcr.io/lixiao189/evoclash-server --listen 0.0.0.0:8080 --public-host <公网 IP>
  ```

- **依赖更新**：Dependabot 每周提 Cargo 和 Actions 的升级 PR；Bevy / lightyear 单独提，需要手动迁移。
