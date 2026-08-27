# ADR-0001：旁路优先的混合采集

- 状态：Accepted
- 日期：2026-08-27

## 背景

产品需要观察现有 Codex Desktop/CLI 会话，同时避免首版成为模型流量故障点。

## 决策

默认读取已持久化 rollout JSONL；用户主动开启时接收官方 OpenTelemetry；当前 App Server 只做本机能力与 Schema 指纹探测，历史读取留给独立 adapter；反代延后为独立可选模块。

## 后果

- 初版无需接管认证和模型流量。
- JSONL adapter 必须容忍内部 Schema 变化。
- OTel 可观察安全化 endpoint，但无反代时不能承诺完整原始 URL、网络 TTFB 或 wire bytes。
- 指标必须展示来源和可信度。
