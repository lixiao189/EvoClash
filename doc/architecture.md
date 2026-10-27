# 架构

Rust · Bevy 0.20（客户端只在浏览器里跑，目标 `wasm32`）· lightyear 0.31（WebTransport + netcode.io）· axum 匹配服务。

```
        网页 + HTTPS/JSON                 WebTransport / QUIC（netcode.io，加密 + 认证）
 浏览器 ───────────────▶ 匹配服务 ──签发令牌──▶ 浏览器 ═══════════════▶ 对局进程（一局一个）
                          ▲   │ 拉起/回收                                      │
                          │   └──────────────▶ evoclash-server                 │
                          └──────────── 每 2 秒心跳（人数、阶段、tick 耗时）◀──┘
```

本地 `fleet` 拉起的对局进程不要带 `--single-match`。Agones 上仍是一个 Pod 一局，由运行时强制 `--single-match`。

| crate | 包名 | 职责 |
|---|---|---|
| `crates/game` | `evoclash-game` | 服务器和客户端共用：协议、确定性模拟、地图、物品、网络常量、匹配 JSON |
| `crates/server` | `evoclash-server` | 无渲染的权威服务器，一个进程一局 |
| `crates/client` | `evoclash` | 浏览器客户端：输入、预测、渲染、特效、HUD、会话。原生 `main` 直接退出，只为了让测试和 clippy 能编 |
| `crates/matchmaker` | `evoclash-matchmaker` | 席位、令牌、房间码、心跳注册表、本地拉起进程，并在 `/play/` 提供网页 |

安装步骤和 Helm 命令仍在仓库根的 [README.md](../README.md)。这里只记设计。

## 共享层 `crates/game`

- `protocol.rs`：复制组件、`PlayerInput`、`GameEvent`、`RoomSettings`、`RoomCommand`。`ProtocolPlugin` 注册通道。新的复制组件必须在这里登记。
- `sim.rs`：移动和战斗规则。服务器权威执行，客户端用同一套函数做本地玩家的预测和回滚。改规则只改这里，不要在服务器或客户端各写一份。不要改 `sim::step_movement` 的签名，除非同时改调用方。
- `map/`：`ArenaMap::generate(seed)` 用 5×3 的手工街区模板（随机镜像，尺寸、门、杂物随机）围着地堡拼一张图。`lot.rs` 的随机和 `rng.rs` 的 SplitMix64 是同一套步进，wasm 和原生逐位一致。生成里禁止 `rand` 和平台相关的浮点函数。`nav.rs` 寻路。`map/tests.rs` 和 `nav.rs` 会扫几百个种子，查墙体重叠、缺角、没有门的建筑和走不到的点。固体之间留 100 像素可走空隙。
- `items/`：护甲、拾取和掉落。`inventory.rs` 是复制的 `Loadout`：枪、枪、近战、投掷，外加 6 格包和每种 `AmmoKind` 一个弹药袋。枪本身没有子弹。`weapons.rs` 里每种武器是一个类型，近战有距离和扇形（客户端的挥砍按这个画），远程有弹道、弹药和 `volley`。线上 id 经 `WeaponKind::arm` / `spec` / `melee` / `ranged` 取回。
- `rng.rs`：`SplitMix64`、`mix`、`stream_seed`。公开房和私人房的对局随机都从这里来。
- `net.rs`：tick 间隔、`PROTOCOL_ID`、netcode 密钥。线上格式一变就要改 `PROTOCOL_ID`，旧客户端在握手时被拒绝。
- `api`（在 `lib.rs`）：匹配服务的 JSON。错误体是 `{error, message, retry_after?}`。

道具（箱子、油桶、保险柜）按 `ArenaMap::props` 生成成复制实体，经 `Obstacles` 传进 `sim::step_movement`，会挡移动。

## 服务器

无头权威进程。插件：连接、玩法、战斗（用位置历史做延迟补偿）、背包（快捷栏、B 面板的 `InventoryMove`、拾取、死亡包；`take` 是唯一发放 `Item` 的函数）、环境（箱子、爆炸桶、保险柜、辐射）、机器人、野怪、运维（心跳和排空）、房间（`gameplay/room.rs`）、日志（`journal.rs`）。

64 Hz 的 `FixedUpdate`，顺序写在 `server/src/gameplay/mod.rs` 的 `SimSet`：`Input → Simulate → Combat → Resolve → Flow`。伤害先放进 `DamageQueue`，在 `Resolve` 结算。向外的事件先进 `Outbox`，在 `Flow` 发出。新系统放进对应的 set。

`FixedUpdate` 使用单线程执行器。Bevy 默认的多线程执行器会把互相冲突的系统跑乱序，同一份输入录下来再重放会对不上。一局一 tick 远小于 1 毫秒，单线程也避免同机几十个进程抢核。

种子在 Playing 开始时重置：`begin_playing` 把 `SimTick` 清零，并把对局随机流按 `map_seed` 重新初始化。Lobby 里发生的事不影响这一局。重放跳过 Lobby，直接从同一 tick 0 重建。

### 日志与离线重放

`logs/` 下两类文本日志（`--log-dir` 可改）：

- `room-<match_id>.log`：房间种子、房间码、是否私人、设置、房主、`match_index`、阶段。每次变化追加一行。给以后的大厅恢复用，现在还不读它。
- `match-<match_id>-<match_index>.log`：Playing 开始时写下 `PROTOCOL_ID`、构建号（`EVOCLASH_BUILD`，否则 `dev`）、种子、设置、按 `client_id` 排序的名单。之后每个 tick 只记外部输入：`PlayerInput`、`PlayerCommand`、`InventoryMove`、`RoomCommand`、连接 / 断线 / 离开。机器人、野怪和掉落不记，靠种子推出来。每 640 tick 一条摘要，哈希按 `client_id` 排序后的位置、血量、分数和经验。Results 结束时写分数、击杀序列和带时间戳的 `end`。

内存缓冲，每 16 tick 追加并 `fsync` 一次。崩溃最多丢 0.25 秒输入。带 `end` 的日志在 16 分钟后删除；没写完的留给崩溃恢复，现在不删。

`--replay <文件>` 不监听端口。用日志头重建 Playing，按 tick 注入输入，逐条比对摘要。第一处不一致打印 `digest mismatch at tick N` 并以 1 退出。8 分钟机器人局的重放要在 15 秒内跑完（发布构建里大约 0.6 秒）。

狗群和巨兽的击杀归属按序号或 `client_id` 排序，不能按实体 id。重放会重新生成实体，id 一定不同。

## 匹配服务

席位预留、签名连接令牌（`tokens.rs`）、心跳喂出来的注册表（`registry.rs`）、房间码（`rooms.rs`）、本地拉起进程（`fleet.rs`；`--no-launch` 只接收外面已经启动的服务器）、在 `/play/` 提供网页（`--web-dir`，默认可执行文件旁边的 `web/`）。

心跳每 2 秒一次，带人数、`max_players`（固定 6）、`tick_us`、证书哈希、阶段、`phase_left_secs`、真人 / 机器人数量、是否私人、房间码、`room_seed`、`match_index`、`join_mode`（`none` 或 `lobby`）、`exiting`。`players` 仍是真人数。匹配服务只信 `join_mode`，不自己重算阶段。主机名会展开成全部解析到的地址（headless Service 的每个副本）。大约 6 秒没有心跳就摘掉。`exiting: true` 表示正常退出前的最后一次心跳，用来和崩溃区分；崩溃后的自动拉起还没做。

快速开始：跳过私人房和 `join_mode != lobby` 的局，按真人数从多到少，再按 `phase_left_secs` 从多到少（`None` 在最后）。

`POST /v1/rooms` 的请求体是 `{name}`，返回带 `room_code` 的加入响应。拉起参数是 `--private --room-code CODE --room-seed N --bots 6`，`--max-players` 固定 6，房间始终补满 6 人；不带 `--single-match`。

## 客户端

输入、本地预测（`predict.rs`）、渲染、特效、HUD、会话和重连（`ehttp` 发 HTTP，WebTransport 连对局）。`index.html` 和 `Trunk.toml` 是网页壳。中文用打包进去的 Noto Sans SC 子集，只有 GB2312。要加子集以外的字，按 `crates/client/assets/README.md` 重新生成字体，不要随手换字体文件。

页面参数：`?name=`（必填）、`?room=`、`?create=1`、`&lag=<ms>`。自己的子弹在点击时本地就画出来，服务器不把它们发回来，命中由服务器判。Starting 的 3 秒里，预测在调用 `sim::step_movement` 之前把移动和动作清掉。

看客户端改动之前要先 `make web` 再刷新。旧的 wasm 构建曾经造成过「改了但页面没变」。

### 画面

玩家是 LPC 精灵。`crates/client/assets/lpc-src/` 放 Universal LPC 生成器的图层。忽略测试 `bake_lpc_sheets` 把它们烤成 `assets/lpc/` 里的表：每种护甲一张身体、一张按玩家颜色着色的白头巾、木棍的正面和背面。`src/lpc/mod.rs` 管切图、每个姿势用哪一帧（走、挥棍、枪是上身前刺加走路的腿、死亡倒地）和各朝向的持枪位置。`render/players.rs` 按纹理像素对世界像素 1:1 画（枪用 `GUN_SCALE`）。程序生成的精灵用 `PIXEL`。姿势预览：`EVOCLASH_POSES_PREVIEW=out.bmp cargo test -p evoclash -- --ignored preview_poses`。

掠夺者用 `lpc/raider.png`（固定衣服、红头巾）加一把步枪，真实武器不复制。狗是 LPC 狼，尸鬼是生成器僵尸。喷吐者和巨兽用 `lpc::BIG_SCALE`。蟑螂、爬行者、无人机、孢子囊仍是程序绘制。箱子、油桶（偏红）、保险柜也是 LPC。道具图是 CC-BY-SA 4.0。

地图同样来自这次烘焙：土、碎进土里的 9 切砾石、人行道、路、墙面（砖和金属高 64，等于 `WALL_HEIGHT`；岩石高 40）、墙顶和两张平屋顶，源图在 `assets/lpc-src/map/`。`render/scenery.rs` 按 1 倍贴，屋顶按 `lpc::ROOF_BORDER` 做 9 切。沙袋、辐射区、地堡地面和室内仍是程序绘制。摄像机有一层灰的 `ColorGrading`，需要 `Tonemapping::Linear`；`None` 会跳过调色。

`lpc/` 里的文件由 Trunk 拷到 `/play/assets/lpc/`，运行时加载。`src/assets.rs` 的 `PendingAssets` 没齐之前，会话不会加入对局。`lpc/` 和 `lpc-src/` 的每个文件都要在 `assets/CREDITS.md` 有一行，否则 `tests/credits.rs` 失败。游戏内说明在 `/play/credits.html`。

程序精灵在启动时画进 `client/src/textures/` 的 `Canvas`（每种一组文件，`mod.rs` 的 `build` 收成 `Textures`），2 倍绘制、最近采样、角色朝 +X。忽略的预览测试写出 BMP，例如 `EVOCLASH_SPRITE_PREVIEW=out.bmp cargo test -p evoclash -- --ignored preview_survivor`。`preview_city` 是地面、屋顶、墙和道具。

`render/` 按 `depth(y)` 排序。同一个角色的部件用很小的 `Z_STEP` 错开。屋顶抬高一个墙的高度，和建筑正面墙一起排序，所以里面的人在屋顶下面。`Indoors` 会藏起本地玩家所不在的那栋建筑里的所有人。

## 复制与延迟

状态每 2 个 tick 复制一次（`REPLICATION_INTERVAL`）。

- **协议**：WebTransport 不可靠数据报（QUIC，没有 TCP 队头阻塞）。输入每个 tick 冗余发送。对局进程启动时生成 14 天自签证书，SHA-256 随心跳到匹配服务，再放进 `JoinResponse.cert_digest`。刚拉起的局要等第一次心跳才能被加入。跑满 12 天会自己排空。浏览器只允许安全页面使用 WebTransport，除了 localhost 以外都要在匹配服务前面加 HTTPS。
- **预测**：本地移动和冲刺没有人为输入延迟。误差靠回滚和平滑修正。预测位置在固定步之间插值。
- **射击**：自己的弹道本地立刻画。命中由服务器决定。
- **延迟补偿**：服务器按每个客户端上报的插值延迟，把目标回溯到「你屏幕上看到的那一刻」再判命中。上限约 220 毫秒（`MAX_REWIND_TICKS` = 14）。
- **调试**：`&lag=80` 在客户端模拟单向延迟。
- **事件**：击杀和公告走可靠有序。命中和爆炸走不可靠通道。`RoomCommand` 和 `PlayerCommand` 走同一条 `CommandChannel`。

## 高并发

- 一局一个进程，固定 6 名战斗者。debug 构建里满编 6 个战斗者加满野怪时 tick 平均约 0.5 毫秒（`/v1/matches` 的 `tick_us`；旧配置实测，6 人为上限），大约是 15.6 毫秒预算的 3%。墙和视线走网格，地图变大不会让每个 tick 变慢。
- 容量等于进程数。匹配服务优先把人放进最满的局。满员（6/6）或正在排空的局不再分配。席位预留防止同时加入时超卖。
- 局与局不共享状态。
- `--threads` 默认 1。
- 预热池：`--warm-matches`（默认 1）个空局。Kubernetes 上由 Agones FleetAutoscaler 的 buffer 负责。
- tick 平均超过约 9.4 毫秒（预算的 60%）的局不再接收新人。

## 高可用

下面是现在已经做到的。崩溃后把同一局拉起来，是 [plan.md](plan.md) 的第 4 期，还没做。

- **对局进程崩溃**：目前只影响这一局。匹配服务大约 6 秒内发现心跳中断并摘除，新人去别的局。本地 `fleet` 能看见子进程退出，但还不会用 `--recover` 重启。
- **匹配服务重启**：对局进程不是它的子进程（`kill_on_drop(false)`），不受影响。新实例靠心跳在大约 2 秒内重新认领。重连密钥是 HMAC，不需要会话存储。注册表本身在内存里，重启会丢掉「正在恢复」这种还没落盘的状态。
- **玩家断线**：角色保留 20 秒，装备和分数都在。客户端退避重连，用重连密钥换新令牌，接回原来的角色。
- **多副本**：`--peer` 列出其他副本，新局向每一个发心跳。每个副本都有完整视图，都能分配。席位预留互不共享，偶尔超卖时由服务器的 netcode 人数上限（6 人）兜底，被拒的客户端会换一局。
- **客户端**：从哪个地址打开页面，就向哪个地址要匹配。原局没了就自动加入新局。
- **排空**：`POST /v1/matches/{id}/drain`，或给进程发 SIGTERM（再发一次立刻退出）。当前局打完再退，不中途踢人。没有真人的局直接退出。匹配服务也响应 SIGTERM。
- **令牌**：匹配服务用共享密钥签名，30 秒过期。名字在加密的 user data 里。生产必须设 `EVOCLASH_NETCODE_KEY`（64 位十六进制），否则用开发密钥。

## Kubernetes

Chart 在 `deploy/helm/evoclash`。匹配服务是 Deployment，带 `--no-launch`，只靠心跳认领对局。对局是 Agones Fleet。`server/src/agones.rs` 通过 sidecar 的 REST 做 Ready、Health、自己 Allocate 和 Shutdown。

| 组件 | 资源 | 要点 |
|---|---|---|
| 匹配服务 | Deployment（3 副本，HPA 3–10）+ LoadBalancer | 打散到不同节点，PDB 至少留 2 个，滚动更新不停服 |
| 心跳 | headless Service `evoclash-matchmaker-heartbeat` | DNS 返回每个匹配 Pod，对局向每一个发心跳 |
| 对局 | Fleet，一局一个 Pod，动态 UDP 端口 | 不报健康就被换掉；SIGTERM 时打完再退 |
| 预热 | FleetAutoscaler，Buffer 3 | 常备 Ready 的空局 |

Pod 崩溃后 `logs/` 会跟着消失，新 Pod 也不一定在同一节点。所以第 5 期如果要在集群里做崩溃恢复，需要 ReadWriteMany 的卷，并且不能靠匹配服务给固定的 Fleet 参数追加 `--recover`。见 [plan.md](plan.md)。

## 发布

同时改 `Cargo.toml` 的 `workspace.package.version` 和 `deploy/helm/evoclash/Chart.yaml` 的 `version` / `appVersion`，合并后推同名 tag `vX.Y.Z`。带 `-` 的 tag 是预发布。`release.yml` 核对 tag 和两处版本，构建网页包 `evoclash-web-vX.Y.Z.tar.gz`（唯一的 Release 附件）和只进镜像的 Linux 服务器 / 匹配服务二进制（镜像里带 `web/`），推 `ghcr.io/lixiao189/evoclash-server`，再把 chart 推到 `oci://ghcr.io/lixiao189/charts/evoclash`。
