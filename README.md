# Okane Strategy Engine

Okane 是一套**稳健、可靠且快速**的量化策略执行与回测平台。它旨在为开发者提供高性能的策略运行环境和安全的执行机制。在确保核心交易系统绝对可靠的前提下，系统预留了接入智能决策模块的能力。

## 🚀 快速开始

### 1. 环境依赖
- **Rust**: 最新稳定版 (Edition 2024)

### 2. 编译与运行
```bash
# 获取源码
git clone https://github.com/moexco/okane.git
cd okane

# 准备配置文件 (参考 config.example.toml)
cp config.example.toml config.toml

# 启动引擎
cargo run --package okane-app
```

## 📖 文档索引

为了方便不同角色的协作，本项目建立了完整的文档体系：

- **[业务产品手册](docs/product.md)**：项目定位、核心能力愿景（AI 交易员、多账号接入）及开发路线图 (Roadmap)。
- **[方案架构说明](docs/architecture.md)**：系统设计真理之源，包含模块依赖图、领域模型（DDD）和核心流程。
- **[开发规范红线](docs/conventions.md)**：开发高质量代码的硬性约束，涵盖 DDD 哲学、Rust 风格及交易系统安全规范。

---

## 🛠 技术栈
- **运行时**: `tokio`
- **数据库**: `sqlx` (SQLite)
- **策略引擎**: `rquickjs` (QuickJS 沙盒)
- **API 框架**: `axum` + `utoipa` (Swagger)
- **金额计算**: `rust_decimal`
