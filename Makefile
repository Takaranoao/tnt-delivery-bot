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

backup: ## 备份 data/ 到 backups/
	@mkdir -p backups
	tar czf backups/tnt-delivery-bot-data-$$(date +%Y%m%d-%H%M%S).tar.gz data
	@echo "已备份到 backups/ (强一致可先 make down 再 backup)"

check: ## 本地 cargo 门禁(可选, 需本机 Rust)
	cargo fmt --all --check
	cargo clippy --all-targets --no-deps -- -D warnings
	cargo test --all-targets --locked
