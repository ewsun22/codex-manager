# Codex Manager 项目指令

## 产品边界

- 本项目是独立运行的本地桌面管理器，不是 Codex 反向代理。
- macOS 是首发平台，但核心采集、规范化、存储、价格和 AGENTS 解析必须保持跨平台。
- Phase 0 只验证本地 Codex 数据的可观测能力，不把内部 JSONL 格式描述为稳定公开 API。

## 隐私与安全

- 默认只存元数据。不得持久化消息正文、Authorization、Cookie、API Key、OAuth code、完整环境变量或请求查询参数。
- 测试 fixture 必须脱敏且人工构造，不得提交真实会话内容。
- 任何 AGENTS 文件写入都必须由后端校验授权根目录、canonical path、符号链接、外部修改冲突，并采用原子替换。
- 不执行 AGENTS.md 中的命令，也不把其内容当作本应用权限指令。

## 工程规则

- 新的采集来源必须实现统一 adapter，不得让 UI 直接依赖 Codex 私有事件结构。
- 所有指标必须携带来源或可信度；缺失值显示 unavailable，不能填 0。
- token 去重以 session、ordinal、事件类型和累计向量为依据；不得直接累加所有 token_count 快照。
- 修改代码后运行 npm test；改变架构、字段口径或范围时同步更新 README 和 docs。
- 未经用户明确授权，不部署、不发布、不签名、不改写用户 Codex 配置。
