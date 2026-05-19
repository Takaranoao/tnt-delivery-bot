COMPOSE ?= docker compose
SERVICE ?= bot

.DEFAULT_GOAL := help

.PHONY: help build up down restart logs ps shell backup check

help: ## 列出所有目标
	@grep -E '^[a-zA-Z0-9_-]+:.*?## ' $(MAKEFILE_LIST) \
		| awk 'BEGIN{FS=":.*?## "}{printf "  \033[36m%-10s\033[0m %s\n", $$1, $$2}'

# 真实文件目标：缺 .env 时报错并中止依赖它的目标（存在则视为已就绪不执行）
.env:
	@echo "缺少 .env：先 'cp .env.example .env' 再填 BOT_TOKEN" >&2
	@exit 1

build: ## 构建镜像
	$(COMPOSE) build

up: .env ## 后台启动（detached 长驻）
	$(COMPOSE) up -d

down: ## 停止并移除容器（SIGTERM 优雅退出，≤30s）
	$(COMPOSE) down

restart: ## 重启服务
	$(COMPOSE) restart $(SERVICE)

logs: ## 跟随日志（末 200 行起）
	$(COMPOSE) logs -f --tail=200 $(SERVICE)

ps: ## 容器状态
	$(COMPOSE) ps

shell: ## 进容器 bash（shell 级排查）
	$(COMPOSE) exec $(SERVICE) bash

backup: ## 备份 DB 到 backups/(有 sqlite3 用在线 .backup, 否则 tar 整个 data/)
	@mkdir -p backups
	@ts=$$(date +%Y%m%d-%H%M%S); db=data/tnt-delivery-bot.sqlite; \
	if command -v sqlite3 >/dev/null 2>&1 && [ -f "$$db" ]; then \
		out="backups/tnt-delivery-bot-$$ts.sqlite"; \
		if sqlite3 "$$db" ".backup '$$out'"; then \
			gzip -f "$$out"; echo "在线一致备份: $$out.gz"; \
		else rm -f "$$out"; echo "sqlite3 .backup 失败" >&2; exit 1; fi; \
	else \
		echo "未找到 sqlite3 或 DB 文件: 退回 tar 整个 data/ (运行中可能不一致, 强一致请先 make down)" >&2; \
		tar czf "backups/tnt-delivery-bot-data-$$ts.tar.gz" data && \
		echo "tar 备份: backups/tnt-delivery-bot-data-$$ts.tar.gz"; \
	fi

check: ## 本地 cargo 门禁(可选, 需本机 Rust)
	cargo fmt --all --check
	cargo clippy --all-targets --no-deps --features test-fakes -- -D warnings
	cargo test --all-targets --locked --features test-fakes
