# [Feature] 本地 LLM 支持 (Local LLM Support)

## 优先级: P1

## 描述

当前仅支持云端 AI API (OpenAI, Anthropic)，需要支持本地 LLM (Ollama, LM Studio)，实现完全离线的 AI 体验。

## 竞品现状

- **Obsidian Copilot**: ✅ Ollama, LM Studio, 本地模型
- **Smart Connections**: ✅ 本地嵌入模型
- **MindWiki**: ❌ 仅云端
- **Notion AI**: ❌ 仅云端
- **NotebookLM**: ❌ 仅云端

## 技术方案

### 方案 1: Ollama 集成 (推荐)

**优点**:
- 最流行的本地 LLM 方案
- 丰富的模型库
- API 兼容 OpenAI
- 社区活跃

**缺点**:
- 需要安装 Ollama
- 模型占用磁盘空间
- 性能依赖硬件

**实现**:
```rust
// src/llm/ollama.rs
pub struct OllamaProvider {
    base_url: String,
    model: String,
}

impl OllamaProvider {
    pub fn new(base_url: &str, model: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
            model: model.to_string(),
        }
    }
}

#[async_trait]
impl LLMProvider for OllamaProvider {
    async fn chat(&self, messages: &[Message]) -> Result<String> {
        let client = reqwest::Client::new();
        let response = client.post(format!("{}/api/chat", self.base_url))
            .json(&serde_json::json!({
                "model": self.model,
                "messages": messages,
                "stream": false,
            }))
            .send()
            .await?;
        
        let body: serde_json::Value = response.json().await?;
        Ok(body["message"]["content"].as_str().unwrap_or("").to_string())
    }
    
    async fn chat_streaming(&self, messages: &[Message]) -> Result<impl Stream<Item = String>> {
        // 流式响应
    }
}
```

### 方案 2: LM Studio 集成

**优点**:
- 图形界面友好
- 模型管理方便
- API 兼容 OpenAI

**缺点**:
- 仅支持桌面
- 非开源
- 模型选择较少

**实现**:
```rust
// src/llm/lmstudio.rs
pub struct LMStudioProvider {
    base_url: String,
    model: String,
}

impl LMStudioProvider {
    pub fn new(base_url: &str, model: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
            model: model.to_string(),
        }
    }
}

#[async_trait]
impl LLMProvider for LMStudioProvider {
    async fn chat(&self, messages: &[Message]) -> Result<String> {
        // 与 Ollama 类似，使用 OpenAI 兼容 API
        let client = reqwest::Client::new();
        let response = client.post(format!("{}/v1/chat/completions", self.base_url))
            .json(&serde_json::json!({
                "model": self.model,
                "messages": messages,
            }))
            .send()
            .await?;
        
        let body: serde_json::Value = response.json().await?;
        Ok(body["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string())
    }
}
```

### 方案 3: 自定义 OpenAI 兼容 API

**优点**:
- 支持任何 OpenAI 兼容服务
- 灵活性高
- 可扩展性强

**缺点**:
- 需要用户配置
- 兼容性问题

**实现**:
```rust
// src/llm/openai_compatible.rs
pub struct OpenAICompatibleProvider {
    base_url: String,
    api_key: Option<String>,
    model: String,
}

impl OpenAICompatibleProvider {
    pub fn new(base_url: &str, api_key: Option<&str>, model: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
            api_key: api_key.map(|k| k.to_string()),
            model: model.to_string(),
        }
    }
}

#[async_trait]
impl LLMProvider for OpenAICompatibleProvider {
    async fn chat(&self, messages: &[Message]) -> Result<String> {
        let client = reqwest::Client::new();
        let mut request = client.post(format!("{}/v1/chat/completions", self.base_url))
            .json(&serde_json::json!({
                "model": self.model,
                "messages": messages,
            }));
        
        if let Some(key) = &self.api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }
        
        let response = request.send().await?;
        let body: serde_json::Value = response.json().await?;
        Ok(body["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string())
    }
}
```

## 实现步骤

### Phase 1: Provider 抽象层 (1 周)

1. **统一 Provider 接口**
   ```rust
   // src/llm/mod.rs
   #[async_trait]
   pub trait LLMProvider: Send + Sync {
       async fn chat(&self, messages: &[Message]) -> Result<String>;
       async fn chat_streaming(&self, messages: &[Message]) -> Result<Pin<Box<dyn Stream<Item = String> + Send>>>;
       async fn list_models(&self) -> Result<Vec<ModelInfo>>;
       fn provider_type(&self) -> ProviderType;
   }
   
   pub enum ProviderType {
       OpenAI,
       Anthropic,
       Ollama,
       LMStudio,
       Custom,
   }
   ```

2. **Provider 工厂**
   ```rust
   // src/llm/factory.rs
   pub fn create_provider(config: &ProviderConfig) -> Result<Box<dyn LLMProvider>> {
       match config.provider_type {
           ProviderType::OpenAI => Ok(Box::new(OpenAIProvider::new(config))),
           ProviderType::Anthropic => Ok(Box::new(AnthropicProvider::new(config))),
           ProviderType::Ollama => Ok(Box::new(OllamaProvider::new(config))),
           ProviderType::LMStudio => Ok(Box::new(LMStudioProvider::new(config))),
           ProviderType::Custom => Ok(Box::new(OpenAICompatibleProvider::new(config))),
       }
   }
   ```

### Phase 2: Ollama 集成 (2 周)

1. **Ollama 服务检测**
   ```rust
   // src/llm/ollama.rs
   impl OllamaProvider {
       pub async fn detect_service() -> Result<Option<OllamaService>> {
           // 1. 检查常见端口 (11434)
           // 2. 尝试连接
           // 3. 获取版本信息
           // 4. 返回服务信息
       }
       
       pub async fn list_local_models(&self) -> Result<Vec<OllamaModel>> {
           // 调用 /api/tags
           // 返回本地模型列表
       }
   }
   ```

2. **模型管理**
   ```rust
   // src/llm/model_manager.rs
   pub struct ModelManager {
       ollama: OllamaProvider,
   }
   
   impl ModelManager {
       pub async fn pull_model(&self, model: &str) -> Result<()> {
           // 调用 /api/pull
           // 显示进度
       }
       
       pub async fn delete_model(&self, model: &str) -> Result<()> {
           // 调用 /api/delete
       }
       
       pub async fn model_info(&self, model: &str) -> Result<ModelInfo> {
           // 调用 /api/show
       }
   }
   ```

3. **嵌入支持**
   ```rust
   // src/embedding/ollama.rs
   impl OllamaProvider {
       pub async fn embed(&self, text: &str, model: &str) -> Result<Vec<f32>> {
           // 调用 /api/embeddings
           // 返回嵌入向量
       }
   }
   ```

### Phase 3: UI 集成 (1 周)

1. **Provider 选择 UI**
   ```csharp
   // SettingsDialog.xaml.cs
   private void OnProviderTypeChanged(object sender, SelectionChangedEventArgs e)
   {
       var providerType = (ProviderType)e.AddedItems[0];
       switch (providerType)
       {
           case ProviderType.Ollama:
               ShowOllamaSettings();
               break;
           case ProviderType.LMStudio:
               ShowLMStudioSettings();
               break;
           // ...
       }
   }
   ```

2. **模型管理 UI**
   ```csharp
   // ModelManagerDialog.xaml.cs
   private async void OnPullModelClicked(object sender, RoutedEventArgs e)
   {
       var model = ModelNameTextBox.Text;
       var progress = new Progress<ModelPullProgress>(UpdateProgress);
       await _modelManager.PullModel(model, progress);
   }
   ```

3. **自动检测 UI**
   ```csharp
   // SettingsDialog.xaml.cs
   private async void OnDetectLocalLLMClicked(object sender, RoutedEventArgs e)
   {
       var services = await DetectLocalServices();
       if (services.Any())
       {
           ShowDetectedServices(services);
       }
       else
       {
           ShowNoServiceDetected();
       }
   }
   ```

### Phase 4: 测试和优化 (1 周)

1. **单元测试**
   ```rust
   #[tokio::test]
   async fn test_ollama_provider() {
       // 模拟 Ollama 服务
       // 测试聊天功能
   }
   
   #[tokio::test]
   async fn test_ollama_streaming() {
       // 测试流式响应
   }
   ```

2. **集成测试**
   ```rust
   #[tokio::test]
   async fn test_ollama_integration() {
       // 启动测试 Ollama 服务
       // 测试完整流程
   }
   ```

3. **性能测试**
   ```rust
   #[bench]
   fn bench_ollama_latency(b: &mut Bencher) {
       // 测试响应延迟
   }
   ```

## CI 测试要求

### 必须通过的测试

```yaml
- name: Local LLM Tests
  run: |
    cargo test ollama_provider
    cargo test lmstudio_provider
    cargo test openai_compatible
    cargo test model_manager
```

### 集成测试 (需要 Ollama 服务)

```yaml
- name: Ollama Integration Tests
  services:
    ollama:
      image: ollama/ollama
      ports:
        - 11434:11434
  run: |
    cargo test --features ollama-integration
```

### 性能基准

```yaml
- name: Local LLM Benchmarks
  run: |
    cargo bench ollama_latency
    cargo bench ollama_throughput
    cargo bench embedding_speed
```

## 成功指标

| 指标 | 目标 | 测量方法 |
|------|------|----------|
| Ollama 检测成功率 | > 95% | 集成测试 |
| 聊天响应延迟 | < 2s (7B 模型) | 性能基准 |
| 嵌入生成速度 | < 100ms/note | 性能基准 |
| 模型切换时间 | < 1s | 集成测试 |
| 内存占用 | < 2GB (7B 模型) | 资源监控 |

## 依赖项

- [ ] Ollama API 文档研究
- [ ] Provider 抽象层设计
- [ ] UI 组件开发
- [ ] 测试环境搭建

## 风险和缓解

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| Ollama API 变更 | 集成失败 | 版本锁定 + 兼容层 |
| 模型质量差 | 回答不准 | 多模型对比 + 回退 |
| 硬件要求高 | 用户无法使用 | 最低配置检测 |
| 内存占用高 | OOM | 模型量化 + 流式加载 |

## 相关 Issue

- #XXX: Provider 抽象层
- #XXX: 嵌入模型集成
- #XXX: 流式响应支持

## 标签

`feature` `P1` `llm` `ollama` `local` `privacy`

## 里程碑

Phase 3: 本地 AI (Week 8-12)
