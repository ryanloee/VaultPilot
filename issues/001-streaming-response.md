# [Feature] 流式响应 (Streaming Response)

## 优先级: P0

## 描述

当前 AI 响应需要等待完整生成后才显示，用户体验差。需要实现流式响应，逐字/逐块显示 AI 回答。

## 竞品现状

- **Obsidian Copilot**: ✅ 支持流式
- **Smart Connections**: ❌ 不支持
- **MindWiki**: ✅ 支持流式
- **Notion AI**: ✅ 支持流式
- **NotebookLM**: ✅ 支持流式

## 技术方案

### 方案 1: Server-Sent Events (SSE)

**优点**:
- 单向通信，简单可靠
- 浏览器原生支持
- 自动重连

**缺点**:
- 单向通信，无法取消
- 需要 HTTP/2 或长连接

**实现**:
```rust
// Rust 后端
async fn stream_response(request: AIRequest) -> impl Stream<Item = String> {
    // 调用 LLM API 的流式接口
    // 逐块返回
}
```

### 方案 2: WebSocket

**优点**:
- 双向通信，支持取消
- 实时性好
- 可复用连接

**缺点**:
- 需要维护连接状态
- 复杂度高

**实现**:
```rust
// Rust 后端
async fn handle_websocket(stream: WebSocket) {
    // 处理流式请求
    // 支持取消指令
}
```

### 方案 3: HTTP Chunked Transfer

**优点**:
- 标准 HTTP，兼容性好
- 无需额外协议
- 简单实现

**缺点**:
- 单向通信
- 需要客户端支持

**实现**:
```rust
// Rust 后端
async fn chunked_response(request: AIRequest) -> Response {
    // 返回 chunked transfer encoding
    // 逐块写入响应体
}
```

### 推荐方案: **HTTP Chunked Transfer**

**理由**:
1. 最简单实现
2. 兼容现有 HTTP 客户端
3. 无需 WebSocket 复杂性
4. 可扩展为 SSE

## 实现步骤

### Phase 1: 后端支持 (1 周)

1. **LLM API 流式调用**
   ```rust
   // src/llm/streaming.rs
   pub async fn call_llm_streaming(
       provider: &dyn LLMProvider,
       request: &AIRequest,
   ) -> Result<impl Stream<Item = String>> {
       // 实现流式调用
   }
   ```

2. **流式响应格式**
   ```rust
   // src/agent/response.rs
   pub enum StreamChunk {
       Text(String),
       Thinking(String),
       Done,
       Error(String),
   }
   ```

3. **HTTP 流式端点**
   ```rust
   // src/api/stream.rs
   pub async fn stream_chat(request: AIRequest) -> Response {
       // 返回 chunked response
   }
   ```

### Phase 2: 前端集成 (1 周)

1. **流式 HTTP 客户端**
   ```csharp
   // BackendClient.cs
   public async IAsyncEnumerable<string> SendStreamingAsync(
       string method,
       object parameters,
       CancellationToken cancellationToken)
   {
       // 使用 HttpClient.SendAsync with HttpCompletionOption.ResponseHeadersRead
       // 逐块读取响应
   }
   ```

2. **UI 流式渲染**
   ```csharp
   // MainWindow.xaml.cs
   private async Task RenderStreamingResponse(
       IAsyncEnumerable<string> chunks,
       CancellationToken cancellationToken)
   {
       await foreach (var chunk in chunks.WithCancellation(cancellationToken))
       {
           // 追加到消息面板
           // 自动滚动到底部
       }
   }
   ```

3. **取消支持**
   ```csharp
   // 通过 CancellationToken 取消流式请求
   ```

### Phase 3: 测试和优化 (1 周)

1. **单元测试**
   ```rust
   #[tokio::test]
   async fn test_streaming_response() {
       // 测试流式解析
   }
   
   #[tokio::test]
   async fn test_streaming_cancellation() {
       // 测试取消
   }
   ```

2. **集成测试**
   ```rust
   #[tokio::test]
   async fn test_streaming_api() {
       // 测试 HTTP 流式端点
   }
   ```

3. **性能测试**
   ```rust
   #[bench]
   fn bench_streaming_latency(b: &mut Bencher) {
       // 测试首字节延迟
   }
   ```

## CI 测试要求

### 必须通过的测试

```yaml
- name: Streaming Tests
  run: |
    cargo test streaming
    cargo test stream_parsing
    cargo test stream_cancellation
    cargo test stream_error_handling
```

### 性能基准

```yaml
- name: Streaming Benchmarks
  run: |
    cargo bench streaming_latency
    cargo bench streaming_throughput
```

### E2E 测试

```yaml
- name: Streaming E2E
  run: |
    # 启动测试服务器
    # 发送流式请求
    # 验证逐块响应
    # 测试取消
```

## 成功指标

| 指标 | 目标 | 测量方法 |
|------|------|----------|
| 首字节延迟 | < 100ms | 基准测试 |
| 流式吞吐量 | > 1000 chars/s | 基准测试 |
| 取消响应时间 | < 200ms | 集成测试 |
| 内存占用 | 无显著增长 | 性能测试 |

## 依赖项

- [ ] LLM Provider 流式 API 支持
- [ ] HTTP 客户端流式读取
- [ ] UI 异步渲染组件

## 风险和缓解

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| LLM API 流式不稳定 | 响应中断 | 自动重试 + 错误恢复 |
| 网络波动 | 流式中断 | 断点续传 + 重连 |
| 内存泄漏 | 性能下降 | 及时释放资源 |
| UI 卡顿 | 体验差 | 节流渲染 + 虚拟滚动 |

## 相关 Issue

- #XXX: 错误恢复机制
- #XXX: 取消功能优化
- #XXX: 性能监控

## 标签

`feature` `P0` `streaming` `ux` `performance`

## 里程碑

Phase 1: 核心体验 (Week 1-3)
