# EvoClash

[![CI](https://github.com/lixiao189/EvoClash/actions/workflows/ci.yml/badge.svg)](https://github.com/lixiao189/EvoClash/actions/workflows/ci.yml)

末日废土背景的俯视角 2D 联机射击游戏，在浏览器里运行。每人拿着木棍出生，搜刮前文明的遗物一路升级到动力装甲和轨道炮；死亡后装备全部掉落。每局 8 名战斗者（真人优先、机器人补位），打满 8 分钟按分数排名，一局接一局循环。

用 Rust 编写：Bevy 0.20（客户端编译为 wasm）、lightyear 0.31（WebTransport + netcode.io）、axum（匹配服务）。

## 开始游玩

需要 [trunk](https://trunkrs.dev)（`cargo install trunk`）。

```bash
make run
```

用 Chrome、Edge 或 Firefox 打开 <http://127.0.0.1:8080>，输入名字即可。多开几个标签页就是多个玩家。

操作：WASD 移动 · 鼠标瞄准、左键射击 · 空格冲刺 · E 拾取（按住撬保险柜） · Q/右键 电磁脉冲雷 · R/T 技能 · Tab 进化面板

## 开发

```bash
make dev     # 调试版客户端，改代码后几秒重建
make watch   # 改客户端代码后自动重建，刷新页面即可
make check   # 本地跑一遍 CI：fmt、clippy（含 wasm）、test
```

## 部署

```bash
helm install evoclash oci://ghcr.io/lixiao189/charts/evoclash -n evoclash --create-namespace
```

浏览器只在 HTTPS 页面上允许 WebTransport，公网部署需要开启带 TLS 的 ingress。参数、端口和 Docker 运行方式见 [doc/deployment.md](doc/deployment.md)。

## 文档

- [玩法与数值](doc/gameplay.md)
- [架构](doc/architecture.md)（网络、高可用、Kubernetes、发布流程）
- [部署](doc/deployment.md)
- [路线图](doc/roadmap.md)

## 许可证

代码采用 MIT OR Apache-2.0。美术素材（`crates/client/assets/lpc/`，来自 [Liberated Pixel Cup](https://lpc.opengameart.org/)）默认按 CC-BY-SA 3.0 分发，见 [LICENSE-ART.md](crates/client/assets/LICENSE-ART.md) 和 [CREDITS.md](crates/client/assets/CREDITS.md)。
