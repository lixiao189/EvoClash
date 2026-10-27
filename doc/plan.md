# 剩余工作

房间流程规格（`room-spec` v3）共五期。前四期已经合进 `main`：

| 期 | 内容 | PR |
|---|---|---|
| 1 | 公开大厅、固定倒计时、每局种子 | [#36](https://github.com/lixiao189/EvoClash/pull/36) |
| 2 | 私人房、房间码、房主设置 | [#37](https://github.com/lixiao189/EvoClash/pull/37) |
| 3 | 事件日志和离线重放 | [#38](https://github.com/lixiao189/EvoClash/pull/38) |
| 4 | 本地 `fleet` 的崩溃恢复 | [#48](https://github.com/lixiao189/EvoClash/pull/48) |

还剩第 5 期（可选，先拍板）。第 4 期的实现见 [architecture.md](architecture.md) 的「崩溃恢复」。玩法规则在 [gameplay.md](gameplay.md)，现有结构在 [architecture.md](architecture.md)。

## 第 5 期（可选）：Kubernetes 上的崩溃恢复

先决定做不做。见下面的开放问题 1。

规格 §9.6。Pod 一死，容器里的 `logs/` 就没了，新 Pod 也不一定调度到同一节点。要做的话：

- 给比赛服务器挂一个 ReadWriteMany 的 PVC（NFS 或 EFS 这类）到 `logs/`。
- Agones Fleet 的启动参数是固定的，匹配服务不能临时加 `--recover`。改成：新分配出来的服务器启动时向匹配服务要「待恢复房间」，再自己到共享卷里找日志。
- 本地 `make run` 的路径不要为了这个改掉。第 4 期的 `--recover` 仍是单机用法。

## 开放问题

这两件规格里还没拍板。没决定之前不要做第 5 期，也不要给匹配服务加数据库。

1. **Kubernetes 上的崩溃恢复做不做？** 需要集群提供 ReadWriteMany。不做的话，崩溃恢复只在 `make run` 和单机部署时生效。
2. **匹配服务自己崩溃时，注册表要不要持久化？** 现在注册表在内存里。它重启后，「正在恢复」的标记会丢。不持久化的话，比赛服务器和匹配服务器同时崩溃的房间无法恢复。只挂一边（对局挂了、匹配还在，或反过来）不受影响。

## 规格之外、README 里仍空着的接口

这些不是房间流程的分期，也没有验收。列在这里避免和上面的工作混在一起。

- **共享席位预留**：多个匹配副本的预留各自在内存里，靠服务器人数上限兜底。要严格不超卖，得把 `Registry` 放到 Redis 一类的地方。
- **全局入口**：多区域现在靠不同域名或 GeoDNS。可以加一个返回区域列表、由网页测速选区的服务。`JoinRequest.region` 已经留了字段。
- **账号与反作弊**：`/v1/join` 前面还没有登录。服务器是权威判定，但没有速率和异常检测。
- **告警**：`/metrics` 已经导出人数、席位、过载局和 tick 耗时。还没有 Prometheus 规则和看板。
