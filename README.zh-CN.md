<div align="center">

<img src="client/public/rlogo.png" alt="LingxiBI logo" width="120" />

# 灵犀BI · LingxiBI

**接入数据库，与数据对话，让 AI 为你构建看板。**

*心有灵犀一点通 —— 你和数据之间的默契。*

<br/>

[![GitHub stars](https://img.shields.io/github/stars/bonfirer/ai-report?style=social)](https://github.com/bonfirer/ai-report/stargazers)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-stable-orange?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![React 19](https://img.shields.io/badge/React-19-61DAFB?logo=react&logoColor=white)](https://react.dev/)
[![Docker](https://img.shields.io/badge/Docker-ready-2496ED?logo=docker&logoColor=white)](#-使用-docker-快速开始推荐)
[![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)](CONTRIBUTING.md)
[![i18n](https://img.shields.io/badge/i18n-EN%20%7C%20中文-9cf)](#-功能特性)

<br/>

[🚀 在线体验](#-在线体验) · [⚡ 快速开始](#-使用-docker-快速开始推荐) · [✨ 功能特性](#-功能特性) · [🧠 自学习机制](#-自学习机制) · [🏗️ 架构](#️-架构) · [📦 部署](#-生产环境部署)

<br/>

⭐ **如果 灵犀BI 对你有帮助，欢迎[在 GitHub 点个 Star](https://github.com/bonfirer/ai-report) —— 这对我们很重要！**

<br/>

[English](README.md) · **简体中文**

</div>

---

## 💡 项目简介

**灵犀BI（LingxiBI）** 是一个会自我学习的 BI 平台，能把原始数据库变成可分享、可交互的数据看板 —— 无需手写 SQL，无需搭建传统 BI 工具。

接入 MySQL、PostgreSQL 或 Oracle，然后：

1. **用自然语言提问** —— AI 自动编写并执行只读 SQL
2. **沉淀指标库** —— 让 AI 始终贴合你的业务定义
3. **越用越聪明** —— 每次对话都会把业务规则教给系统
4. **生成 HTML 看板** —— 通过对话持续优化，完整版本历史可回滚
5. **及时获知变化** —— 阈值预警通过邮件（附带 Excel）和/或飞书卡片推送

<br/>

<div align="center">
  <img src="client/public/report.png" alt="AI 生成的看板" width="720" />
  <br/><sub>▲ AI 一键生成的可交互数据看板（ECharts）</sub>
</div>

<br/>

<div align="center">
  <img src="client/public/data.png" alt="数据对话" width="720" />
  <br/><sub>▲ 用自然语言与数据源对话</sub>
</div>

---

## 🚀 在线体验

<table>
  <tr><td>🌐 <b>网址</b></td><td><a href="https://www.termiio.com:9528">https://www.termiio.com:9528</a></td></tr>
  <tr><td>👤 <b>用户名</b></td><td><code>admin</code></td></tr>
  <tr><td>🔑 <b>密码</b></td><td><code>admin123</code></td></tr>
</table>

> ⚠️ 公开共享演示环境 —— 请勿录入真实凭据或敏感数据，数据可能被定期重置。

---

## ✨ 功能特性

| | 能力 | 说明 |
|---|---|---|
| 🔌 | **多源接入** | MySQL · PostgreSQL · Oracle —— 自动解析表结构，可视化表间关系为知识图谱 |
| 💬 | **AI 对话** | 自然语言 → 只读 SQL，自动修复失败查询，服务端流式生成（切走页面也不中断） |
| ⭐ | **指标库** | 验证过的命名业务指标 —— 同时作为 AI 知识库 |
| 📊 | **AI 看板** | 一句话生成响应式 ECharts HTML 看板，对话式迭代，版本历史 + 回滚 |
| 🎨 | **收藏主题** | 把报表的视觉风格（配色、排版、图表样式）存为可复用主题，一键用该风格生成新看板 |
| 💡 | **AI 数据分析总结** | 一键生成报表的叙述式分析——核心结论、关键发现、趋势、异常与行动建议，基于真实数据、快照趋势和知识库，禁止编造（可缓存、可重新生成） |
| 🧠 | **自学习** | 自动提炼按数据源隔离的业务知识；👍 转 few-shot 示例；相关性排序召回 |
| 📸 | **快照** | 定时采集指标快照，支持趋势/同比/环比分析 |
| 🔔 | **多渠道预警** | 阈值规则 → AI 撰写的邮件（附 Excel）和/或飞书交互卡片（HMAC-SHA256 加签） |
| 🔗 | **分享** | 不可猜测的分享链接，支持发布/草稿控制 |
| 👥 | **多用户与权限** | 管理员/成员两种角色；报表、指标、会话、预警按用户隔离；数据源级访问授权 |
| 🌍 | **国际化** | 内置中英文界面 |
| 🔐 | **安全** | JWT 认证、登录限流、SQL 白名单校验、SSRF 防护、安全响应头 |

---

## 🧠 自学习机制

大多数 text-to-SQL 工具是无状态的——问完即忘。灵犀BI 则会**不断沉淀按数据源隔离的业务知识**，并回流到每一次回答里。团队用得越多，它就越准。

```mermaid
flowchart TD
    A(["🧑 你 — 提问 / 👍 点赞"]) --> B["💬 对话\n(WebSocket 流式)"]
    B --> C["⚙️ 后台任务：\n提炼新增业务知识"]
    C --> D[("📚 已学习上下文\n(按数据源隔离)")]
    D --> E["🎯 相关性排序 +\ntoken 预算控制"]
    E --> F["✨ 有据可依的 SQL 生成\n→ 下一次回答更好"]
    F -.->|"回流到下次对话"| B

    D --- K1["🔗 知识库\n表关系 · 规则 · 字段含义"]
    D --- K2["👍 few-shot 示例"]
    D --- K3["⭐ 精选指标库"]
    D --- K4["📊 列画像\n枚举值 · 范围 · 样本"]
```

**核心机制：**

- **自动抽取** —— 每轮对话后，LLM 只提炼新增知识（与已知去重，带置信度）
- **人在回路** —— 对好的回答点 👍，存为下次模型会遵循的 few-shot 示例
- **精选指标** —— 经过验证的命名 SQL，作为高可信的可复用定义
- **智能召回** —— 按关键词相关性 × 置信度排序，限制在 token 预算内

---

## 🛠️ 技术栈

| 层 | 技术 |
|-------|------------|
| 🦀 **后端** | Rust · [Axum](https://github.com/tokio-rs/axum) · [SQLx](https://github.com/launchbadge/sqlx) · [Tokio](https://tokio.rs/) |
| 🗄️ **元数据库** | MySQL / MariaDB |
| 🎯 **数据源** | MySQL · PostgreSQL · Oracle |
| 🤖 **大模型** | 任意 OpenAI 兼容 API（DeepSeek、GPT-4o、Qwen 等） |
| 📧 **投递** | SMTP ([lettre](https://github.com/lettre/lettre)) · Excel ([rust_xlsxwriter](https://github.com/jmcnamara/rust_xlsxwriter)) · 飞书 webhook (HMAC-SHA256) |
| ⚛️ **前端** | React 19 · Vite · TypeScript · Tailwind CSS · Zustand · React Router |

---

## 🏗️ 架构

```mermaid
flowchart TD
    Browser["🌐 浏览器"] -->|HTTPS| Nginx["Nginx :9528"]
    
    Nginx -->|"/"| SPA["⚛️ 静态 SPA\n(client/dist)"]
    Nginx -->|"/api/*"| API["🦀 Rust API\n(Axum :3001)"]
    Nginx -->|"/api/chat"| WS["🔌 WebSocket"]

    API --> MetaDB[("🗄️ MySQL/MariaDB\n元数据库")]
    API --> DS["🎯 数据源\nMySQL · PG · Oracle"]
    API --> LLM["🤖 大模型服务\nOpenAI 兼容"]

    MetaDB --> Scheduler["⏰ 后台调度器"]
    Scheduler --> Snap["📸 快照"]
    Scheduler --> Alerts["🔔 预警"]
    Alerts --> SMTP["📧 SMTP\n邮件 + Excel"]
    Alerts --> Feishu["🪶 飞书\n交互式卡片"]
```

**设计原则：**

- **无状态 API** —— 只有元数据库持有状态，水平扩展友好
- **原子调度** —— 后台任务原子领取，多实例安全
- **多渠道投递** —— 每条规则可选邮件、飞书或两者同时；各渠道结果独立记录
- **安全优先** —— SQL 白名单校验、Webhook SSRF 防护、bcrypt-12、安全响应头

---

## ⚡ 使用 Docker 快速开始（推荐）

```bash
docker compose up -d --build
```

打开 **http://localhost:9528** → 创建首个管理员账户，即可使用。

| 服务 | 角色 |
|---------|------|
| `db` | MySQL 元数据存储（仅内部访问） |
| `server` | Rust API `:3001`（首次启动自动生成 JWT_SECRET） |
| `web` | Nginx `:9528` —— SPA + API 代理（含 WebSocket） |

```bash
docker compose logs -f server     # 跟踪 API 日志
docker compose down               # 停止（保留数据）
docker compose down -v            # 停止并清除所有数据
```

> 🛡️ **生产环境：** 在 `.env` 中设置强密码，`CORS_ALLOWED_ORIGIN` 改为真实域名，并在上游做 TLS 终止。

---

## 💻 本地开发

<details>
<summary><b>前置依赖</b></summary>

- [Rust](https://rustup.rs/)（stable）
- [Node.js](https://nodejs.org/) ≥ 18
- MySQL 或 MariaDB

</details>

```bash
# 1. 创建元数据库
mysql -e "CREATE DATABASE ai_report CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;"

# 2. 启动后端
cd server
cp .env.example .env   # 编辑 DATABASE_URL + JWT_SECRET
cargo run              # 自动执行迁移，监听 :3001

# 3. 启动前端
cd client
npm install && npm run dev   # Vite 代理 /api → :3001
```

打开终端输出的地址，创建管理员，添加数据源，在**设置**中配置大模型。

---

## ⚙️ 配置

| 变量 | 说明 |
|----------|-------------|
| `DATABASE_URL` | 元数据库连接串 |
| `JWT_SECRET` | Token 签名密钥（≥ 16 字符） |
| `CORS_ALLOWED_ORIGIN` | 允许的来源（开发用 `*`） |

> 大模型、SMTP、飞书 Webhook 均在应用内运行时配置（存储在数据库，非环境变量）。

---

## 📦 生产环境部署

```bash
# 服务器一次性初始化（Rust、MySQL、Nginx、TLS）：
bash scripts/setup-server.sh [domain]

# 从本地发布：
./scripts/deploy.sh user@host [domain]
```

Rust 二进制在目标主机编译（避免 glibc 不匹配）。SPA 在本地构建后作为静态文件部署。

> 💡 Docker Compose 方案同样可用于生产环境，部署在 TLS 代理之后。

---

## 🗂️ 项目结构

```
lingxibi/
├── client/                  React + Vite SPA
│   └── src/
│       ├── pages/           路由级页面
│       ├── components/      通用 UI 组件
│       ├── stores/          Zustand 状态管理
│       ├── lib/             API 客户端与类型
│       └── i18n/            中英文翻译
├── server/                  Rust (Axum) API
│   ├── src/
│   │   ├── routes/          HTTP + WebSocket 处理器
│   │   ├── llm/             大模型客户端 + 提示工程
│   │   ├── alert_engine.rs  预警评估 + 多渠道投递
│   │   ├── feishu.rs        飞书 Webhook + HMAC 加签
│   │   ├── email.rs         SMTP 邮件发送
│   │   └── ...
│   └── migrations/          SQL 迁移（自动执行）
├── scripts/                 部署自动化
├── docker-compose.yml       一条命令拉起全栈
└── .env.example             环境变量模板
```

---

## 🔒 安全

| 层面 | 机制 |
|------|------|
| **认证** | JWT 会话、bcrypt-12、登录限流（5 次失败 / 5 分钟锁定） |
| **授权** | 管理员/成员两种角色；报表、指标、会话、预警按用户归属隔离；数据源级访问授权（每次读取/查询均在服务端强制校验） |
| **SQL 安全** | 词法白名单校验（仅允许 SELECT/SHOW/DESCRIBE/EXPLAIN/CTE）、单查询超时 30s、行数上限 50k |
| **SSRF 防护** | 飞书 Webhook URL 限制为官方域名 |
| **响应头** | `X-Content-Type-Options`、`Referrer-Policy`、`Permissions-Policy` |
| **凭据保护** | API 永远不返回密钥/密码；响应中始终掩码 |

> 📣 安全漏洞请私信 **[macrogroot@outlook.com](mailto:macrogroot@outlook.com)**，不要公开提交 issue。

---

## 👥 多用户与权限

灵犀BI 支持多用户，分为两种角色，并明确区分「共享基础设施」与「个人产出」。

**角色**
- **管理员（admin）** —— 管理共享基础设施，可查看/管理一切：数据源、AI 服务商（LLM）配置、SMTP/飞书设置、知识库以及用户账户。
- **成员（member）** —— 只能操作自己的内容，以及被授权的数据源。

**私有 vs. 共享**

| 资源 | 可见范围 |
|------|----------|
| 报表 · 指标 · 会话 · 预警规则 · 快照计划 · 主题 | **按用户隔离** —— 成员只看到自己的，管理员看到全部 |
| 数据源 · 结构 · 知识库 · AI 示例 · LLM/SMTP/飞书配置 | **共享** —— 有权限者可读，仅管理员可增删改 |

**数据源级授权**
数据源由管理员统一维护。成员在被管理员授权前，看不到也无法查询该数据源。授权在服务端对所有涉及取数的路径强制校验 —— 列表、结构/知识图谱读取、即席 SQL、指标与报表数据集的创建/刷新、快照、以及 AI 对话。撤销授权后立即阻止后续查询（已缓存的数据在被覆盖前仍保留）。管理员默认拥有所有数据源的访问权限。

**首次运行与新增用户**
- 首次注册的账户即为**管理员**，之后注册入口关闭。
- 在 **设置 → 用户管理**（仅管理员）新增成员/管理员、重置密码、切换角色。
- 在 **数据源 → 选中某数据源 → 访问权限** 为成员授权。

> 已有的单管理员部署：升级后所有存量数据归属到第一个管理员，数据源初始无成员授权 —— 随着新增成员按需授权即可。

---

## 🗺️ 路线图

- [ ] 基于 embedding 的语义检索（知识库与示例）
- [ ] 飞书多维表格（Bitable）同步
- [ ] 更多通知渠道（钉钉、企业微信、Slack）
- [ ] 凭据静态加密
- [ ] 多架构 Docker 镜像（GHCR）
- [ ] `SECURITY.md` + `CHANGELOG.md`
- [ ] 更多图表类型与看板模板

---

## 🤝 参与贡献

欢迎贡献！请阅读 [CONTRIBUTING.md](CONTRIBUTING.md) 了解开发流程和规范。

## 📬 联系方式

- 📧 [macrogroot@outlook.com](mailto:macrogroot@outlook.com)
- 🐛 [提交 Issue](../../issues)

## 📄 许可证

[MIT License](LICENSE) © 2026 Macro

<div align="center">
<br/>
<sub>用 🦀 Rust 与 ⚛️ React 构建 · Powered by AI · Ethan</sub>
</div>
