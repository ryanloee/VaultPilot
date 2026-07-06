# 移动端 Agent Mode 技术方案调研

> Issue: #1360 | Date: 2026-07-06 | Author: pipeline-developer
> 
> Agent Mode Phase 3.2 已在 Rust 后端 + CLI + WinUI 三端实现。移动端（Expo 56 + React Native 0.85.3）Agent Mode 完全空白。

---

## 1. 现状分析

### 1.1 后端现状
- Rust 后端: `src/agent.rs` — 完整的沙箱 Agent，含权限模型 (`AgentPermission`)、资源限制 (`AgentResourceLimits`)、ToolProxy、审计日志
- Agent Engine 层: `src/agent_engine.rs` — 统一接口支持内置 agent + 外部 CLI agent（Claude Code, Codex）
- HTTP 层: axum 0.8 已在 `Cargo.toml` 中，支持 `http1/json/multipart`
- 已有 `http_bridge.rs` — REST 服务框架（LLM proxy 等）

### 1.2 移动端现状
- Expo 56 + React Native 0.85.3 + TypeScript 6.0，**managed workflow**（已 `prebuild` 有 `android/` 目录）
- 已连接远程后端: `src/api/client.ts`、`src/api/sse.ts`（SSE streaming with reconnect）
- 安全存储: `expo-secure-store`（API keys）
- 离线队列: `src/services/sync.ts`、`src/utils/offlineSync.ts`
- UI 已有: AI Action Palette、Chat 界面、Note Editor、Search

---

## 2. 方案对比

### 方案 A：本地 Rust FFI

| 维度 | 评估 |
|------|------|
| **技术路线** | Mozilla `uniffi-bindgen-react-native` v0.15（JSI + Turbo Module）|
| **开发成本** | ~10-14 周（1 人）|
| **离线可用** | ✅ 完全离线，所有代码本地运行 |
| **延迟** | ✅ ~0.5-5ms per FFI call |
| **安全** | ✅ ToolProxy 权限模型直接复用 |
| **兼容性** | ⚠️ 需要 `expo prebuild` + Android NDK + Xcode；损失 Expo Go 兼容（已损失）|
| **App 大小** | ⚠️ +5-15MB（Rust 静态库）|
| **维护风险** | 🔴 `uniffi-bindgen-react-native` 是 v0.15（pre-1.0），API 可能变更 |
| **测试** | ⚠️ 无法在 Expo Go 中测试，需要真机/模拟器 |

**Pros:**
- 单套 Rust 代码库同时服务 desktop + mobile agent
- 安全模型 (ToolProxy) 直接转移
- 已验证的生产路径（Matrix SDK, ChessTiles）

**Cons:**
- 构建链复杂: cargo-ndk + NDK + Xcode + CI
- iOS 需要 macOS（或 EAS Build）
- 桥接库尚未 GA

### 方案 B：远程服务器模式

| 维度 | 评估 |
|------|------|
| **技术路线** | 在 axum 中添加 `/api/agent/*` 路由，移动端通过 REST/SSE 调用 |
| **开发成本** | ~3-4 周（混合 Phase 1）/ ~6 周（完整）|
| **离线可用** | ❌ 离线无法执行 agent（需 LLM API / agent CLI）|
| **延迟** | ⚠️ 依赖网络，比 FFI 高 50-500ms |
| **安全** | ✅ 服务端已有多租户 bearer token 模式；SecureStore 存储 |
| **代码复用** | ✅ AgentConfig, AgentResourceLimits, ToolProxy 全部直接复用 |
| **移动端改动** | ✅ 最小改动 — 现有 REST/SSE client 可扩展 |
| **开发风险** | ✅ 低 — 成熟技术栈 |

**Pros:**
- 现有 axum 基础设施直接使用
- 移动端改动最小，保持 Expo managed workflow
- 所有 agent 逻辑在服务端，避免性能/电池影响
- 现有 SSE streaming 实现可复用 (`parseSSEStreamWithReconnect`)

**Cons:**
- 需要网络连接
- 离线时 agent 不可用（但可降级到本地搜索/摘要）

**推荐 REST API 设计：**

```
POST   /api/agent/sessions              — 创建 agent session
POST   /api/agent/sessions/{id}/prompt  — 发送 prompt（SSE 流式返回）
POST   /api/agent/sessions/{id}/plan/decision — Plan Mode 决策
DELETE /api/agent/sessions/{id}         — 取消/删除 session
GET    /api/agent/engines               — 列举可用 engine
GET    /api/agent/sessions/{id}         — 查询 session 状态
```

### 方案 C：混合模式 ⭐ **推荐**

| 维度 | 评估 |
|------|------|
| **技术路线** | 简单任务设备端执行 + 复杂任务远程 agent |
| **开发成本** | ~6-8 周（完整三阶段）|
| **离线可用** | ✅ 简单任务完全离线（搜索、总结、分类）|
| **延迟** | ✅ 简单任务即时；复杂任务走网络 |
| **安全** | ✅ 写操作始终走服务端 Rust 沙箱 |
| **设备端能力** | `react-native-executorch` — 无需 eject，Expo prebuild 即可 |
| **可测试性** | ✅ 可分阶段交付 |

**Pros:**
- 70% 日常任务离线可用（搜索、摘要、分类）
- 写操作/多步 agent 始终走服务端沙箱，安全不妥协
- 分阶段交付，早期即可获得价值
- 跟随 Apple Intelligence 已验证模式（on-device → escalate to cloud）

**Cons:**
- 架构复杂度最高（两条路径）
- 需要维护两种执行模式
- 设备端模型量化需要额外调优

---

## 3. 决策矩阵

| 标准 | 权重 | A. Rust FFI | B. Remote | C. Hybrid |
|------|------|:-----------:|:---------:|:---------:|
| 开发成本与周期 | 25% | 2 (50) | **4 (100)** | 3 (75) |
| 性能/延迟/电量 | 20% | **5 (100)** | 3 (60) | 4 (80) |
| 离线可用性 | 20% | **5 (100)** | 1 (20) | 4 (80) |
| 安全/与现有架构兼容 | 20% | 4 (80) | **5 (100)** | **5 (100)** |
| 维护风险与可测试性 | 15% | 2 (30) | **4 (60)** | 3 (45) |
| **加权总分** | 100% | **3.45** | 3.25 | **3.80** ✅ |

---

## 4. 推荐实施路线图

### Phase 1（3-4 周）— 远程 Agent Baseline
1. **后端 (2 周)**: 在 `http_bridge.rs` 中实现 `/api/agent/*` REST API
   - 复用 `AgentConfig`, `AgentResourceLimits`, `AgentPermission`
   - SSE streaming for agent events
   - Cancellation token 支持
2. **移动端 (1-2 周)**: 
   - 在 `src/api/client.ts` 中添加 agent 端点
   - 复用 `parseSSEStreamWithReconnect` 流式展示
   - Agent mode 入口 UI（Action Palette 扩展或独立界面）

### Phase 2（2 周）— 设备端简单任务
1. 集成 `react-native-executorch`（Expo prebuild, no eject）
2. 部署量化模型（LFM 2.5 1.2B 或 Gemma 2B）
3. 实现本地搜索/摘要/分类能力
4. 智能路由: 简单任务走本地，复杂/写操作走远程

### Phase 3（2 周）— 智能层
1. 离线队列: agent prompts 入队，网络恢复后自动执行
2. 智能切换: 根据网络状态/任务复杂度自动选择执行路径
3. 性能调优: 设备端模型量化、内存优化

---

## 5. 竞品参考

| 产品 | 策略 | 启示 |
|------|------|------|
| **Notion AI 3.2** | 全云，离线不支持 AI | ⚠️ 避免此限制 |
| **Obsidian Copilot v4** | Desktop only，无移动 agent | 📌 VaultPilot 差异化机会 |
| **Apple Intelligence** | ~3B on-device + PII 云升级 | ✅ 已验证混合模式 |
| **GitHub Copilot Mobile** | 全云，代码补全 + 聊天 | ⚠️ 与 Obsidian 不同赛道 |

---

## 6. 结论

**推荐方案：混合模式（方案 C）**

理由：
1. 70% 日常任务搜索/摘要/分类可在设备端离线完成
2. 写操作/多步 agent 始终走服务端 Rust 沙箱，安全不妥协
3. 分阶段交付：Phase 1 (远程 baseline) 仅需 3-4 周即可获得初始价值
4. 保持 Expo managed workflow（无需 eject）
5. 遵循 Apple Intelligence 已验证的混合模式

**下一步**: Phase 1 — 在 axum 中实现 agent REST API 并连接移动端 UI。
