# tnt-delivery-bot

大统华(T&T)配送通知 Telegram 机器人。发 token 或追踪链接给它,订单字段变化时推送给你。

## 用法

发给机器人下列任一形式即可开始追踪同一订单:

- `3abc126`
- `https://tmstracking.tntsupermarket.us/#/3abc156`
- `https://tmsapi.tntsupermarket.us/track/customer?token=3abc856`
- `Your T&T order 0000752 is on the way. https://tmstracking.tntsupermarket.us/#/3a28856 [Do Not Reply]`

命令: `/list` 查看在追订单 · `/stop <token>` 停止追踪 · `/help`

## 运行

### 本地

```bash
cp .env.example .env
$EDITOR .env   # 至少填 BOT_TOKEN
cargo run --release
```

### Docker

```bash
cp .env.example .env
$EDITOR .env   # 至少填 BOT_TOKEN
make up        # 构建并后台长驻
make logs      # 跟随日志
make down      # 优雅停止(SIGTERM, ≤30s)
```

数据持久化在 `./data/`（SQLite + WAL）；容器以非 root 运行，`./data` 在宿主上
属主会被设为 uid `10001`。`make backup` 输出到 `backups/`（宿主有 `sqlite3` 时
走在线一致备份，否则 tar）；`make help` 看全部目标。

token 最多保留 24h(任何人再次发送即续期);加入时若订单状态为 `UNKNOWN` 则不加入并提示;
订单状态变为 `COMPLETED` 时,推送本次变化后自动停止追踪并通知(加入时若已是 `COMPLETED` 则不加入并提示);
连续查询失败 `MAX_FETCH_FAILURES` 次后自动停止并通知。
