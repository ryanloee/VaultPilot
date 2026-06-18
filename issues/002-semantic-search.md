# [Feature] 语义搜索 (Semantic Search)

## 优先级: P0

## 描述

当前仅有 FTS5 关键词搜索，无法理解语义相似性。需要实现向量语义搜索，支持自然语言查询和语义匹配。

## 竞品现状

- **Obsidian Copilot**: ✅ 语义搜索 + RAG
- **Smart Connections**: ✅ 本地向量嵌入
- **MindWiki**: ✅ 语义搜索
- **Notion AI**: ✅ 语义搜索
- **NotebookLM**: ✅ 多源语义搜索

## 技术方案

### 方案 1: 本地向量库 (sqlite-vss)

**优点**:
- 与现有 SQLite 集成
- 无需额外服务
- 部署简单

**缺点**:
- 性能可能不如专用向量库
- 功能相对简单

**实现**:
```sql
-- 安装 sqlite-vss 扩展
CREATE VIRTUAL TABLE note_embeddings USING vss0(
    embedding(384)  -- nomic-embed-text 维度
);

-- 插入嵌入
INSERT INTO note_embeddings (rowid, embedding) 
VALUES (1, vector_from_json('[0.1, 0.2, ...]'));

-- 语义搜索
SELECT n.* FROM notes n
JOIN note_embeddings e ON n.id = e.rowid
WHERE vss_search(e.embedding, vector_from_json('[0.1, 0.2, ...]'))
LIMIT 10;
```

### 方案 2: 专用向量库 (Qdrant)

**优点**:
- 高性能向量搜索
- 丰富的过滤功能
- 可扩展性好

**缺点**:
- 需要额外服务
- 部署复杂
- 资源占用高

**实现**:
```rust
// 使用 Qdrant 客户端
use qdrant_client::Qdrant;

let client = Qdrant::from_url("http://localhost:6334").build()?;

// 创建集合
client.create_collection(CreateCollection {
    collection_name: "notes".to_string(),
    vectors_config: Some(VectorsConfig::Params(VectorParams {
        size: 384,
        distance: Distance::Cosine.into(),
    })),
    ..Default::default()
}).await?;

// 插入向量
client.upsert_points(UpsertPoints {
    collection_name: "notes".to_string(),
    points: vec![PointStruct {
        id: Some(point_id),
        vectors: Some(Vectors::from(embedding)),
        payload: HashMap::from([("title".into(), "note title".into())]),
    }],
    ..Default::default()
}).await?;

// 搜索
let results = client.search_points(SearchPoints {
    collection_name: "notes".to_string(),
    vector: query_embedding,
    limit: 10,
    ..Default::default()
}).await?;
```

### 方案 3: 混合方案 (推荐)

**优点**:
- 关键词精确匹配
- 语义相似匹配
- 结果融合优化

**缺点**:
- 实现复杂
- 需要调优权重

**实现**:
```rust
// 混合搜索
pub async fn hybrid_search(
    query: &str,
    fts_weight: f32,
    vector_weight: f32,
) -> Result<Vec<SearchResult>> {
    // 1. FTS5 关键词搜索
    let fts_results = fts_search(query).await?;
    
    // 2. 向量语义搜索
    let vector_results = vector_search(query).await?;
    
    // 3. 结果融合
    let merged = merge_results(fts_results, vector_results, fts_weight, vector_weight);
    
    Ok(merged)
}
```

## 实现步骤

### Phase 1: 嵌入基础设施 (2 周)

1. **嵌入模型集成**
   ```rust
   // src/embedding/mod.rs
   pub trait EmbeddingModel {
       async fn embed(&self, text: &str) -> Result<Vec<f32>>;
       fn dimension(&self) -> usize;
   }
   
   // 支持多种嵌入模型
   pub struct NomicEmbedText;
   pub struct OpenAIEmbedding;
   pub struct LocalEmbedding;
   ```

2. **向量存储**
   ```rust
   // src/storage/vector.rs
   pub struct VectorStore {
       db: Connection,
   }
   
   impl VectorStore {
       pub async fn index_note(&self, note_id: &str, content: &str) -> Result<()> {
           let embedding = self.model.embed(content).await?;
           self.insert_embedding(note_id, &embedding).await?;
           Ok(())
       }
       
       pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
           let query_embedding = self.model.embed(query).await?;
           let results = self.vector_similarity(&query_embedding, limit).await?;
           Ok(results)
       }
   }
   ```

3. **索引管理**
   ```rust
   // src/indexing/mod.rs
   pub struct IndexManager {
       vector_store: VectorStore,
       embedding_model: Box<dyn EmbeddingModel>,
   }
   
   impl IndexManager {
       pub async fn rebuild_index(&self) -> Result<()> {
           // 1. 获取所有笔记
           // 2. 生成嵌入
           // 3. 存储到向量库
           // 4. 更新索引
       }
       
       pub async fn update_note(&self, note_id: &str) -> Result<()> {
           // 增量更新
       }
   }
   ```

### Phase 2: 搜索 API (1 周)

1. **混合搜索 API**
   ```rust
   // src/search/hybrid.rs
   pub async fn search_notes(
       query: &str,
       filters: SearchFilters,
       options: SearchOptions,
   ) -> Result<SearchResponse> {
       // 1. 解析查询意图
       let intent = parse_query_intent(query);
       
       // 2. 根据意图选择搜索策略
       let results = match intent {
           QueryIntent::Keyword => fts_search(query, filters).await?,
           QueryIntent::Semantic => vector_search(query, filters).await?,
           QueryIntent::Hybrid => hybrid_search(query, filters, options.weights).await?,
       };
       
       // 3. 后处理 (去重, 排序, 分页)
       let processed = post_process(results, options);
       
       Ok(processed)
   }
   ```

2. **相关笔记 API**
   ```rust
   // src/search/related.rs
   pub async fn get_related_notes(
       note_id: &str,
       limit: usize,
   ) -> Result<Vec<RelatedNote>> {
       // 1. 获取当前笔记嵌入
       let embedding = get_note_embedding(note_id).await?;
       
       // 2. 查找相似笔记
       let similar = vector_search_by_embedding(&embedding, limit + 1).await?;
       
       // 3. 排除自身
       let related = similar.into_iter()
           .filter(|r| r.note_id != note_id)
           .take(limit)
           .collect();
       
       Ok(related)
   }
   ```

### Phase 3: UI 集成 (1 周)

1. **搜索增强 UI**
   ```csharp
   // SearchView.xaml.cs
   private async void OnSearchQueryChanged(string query)
   {
       // 显示搜索建议
       var suggestions = await GetSearchSuggestions(query);
       ShowSuggestions(suggestions);
   }
   
   private async void OnSearchExecuted(string query)
   {
       // 执行混合搜索
       var results = await SearchNotesHybrid(query);
       DisplaySearchResults(results);
   }
   ```

2. **相关笔记面板**
   ```csharp
   // RelatedNotesPanel.xaml.cs
   private async void OnNoteSelected(string noteId)
   {
       // 获取相关笔记
       var related = await GetRelatedNotes(noteId);
       DisplayRelatedNotes(related);
   }
   ```

### Phase 4: 测试和优化 (1 周)

1. **准确性测试**
   ```rust
   #[tokio::test]
   async fn test_semantic_search_accuracy() {
       // 测试语义相似查询
       let results = search_notes("机器学习算法").await?;
       assert!(results.iter().any(|r| r.title.contains("深度学习")));
   }
   ```

2. **性能测试**
   ```rust
   #[bench]
   fn bench_vector_search(b: &mut Bencher) {
       // 测试搜索延迟
   }
   
   #[bench]
   fn bench_embedding_generation(b: &mut Bencher) {
       // 测试嵌入生成速度
   }
   ```

3. **混合搜索调优**
   ```rust
   #[tokio::test]
   async fn test_hybrid_search_weights() {
       // 测试不同权重组合
       let results1 = hybrid_search("query", 0.7, 0.3).await?;
       let results2 = hybrid_search("query", 0.3, 0.7).await?;
       // 比较结果质量
   }
   ```

## CI 测试要求

### 必须通过的测试

```yaml
- name: Semantic Search Tests
  run: |
    cargo test embedding
    cargo test vector_store
    cargo test semantic_search
    cargo test related_notes
    cargo test hybrid_search
```

### 性能基准

```yaml
- name: Search Benchmarks
  run: |
    cargo bench embedding_generation
    cargo bench vector_search
    cargo bench hybrid_search
    cargo bench index_rebuild
```

### 准确性测试

```yaml
- name: Search Accuracy Tests
  run: |
    cargo test search_accuracy
    cargo test recall_rate
    cargo test precision_rate
```

## 成功指标

| 指标 | 目标 | 测量方法 |
|------|------|----------|
| 语义搜索召回率 | > 80% | 准确性测试 |
| 搜索延迟 | < 200ms | 性能基准 |
| 嵌入生成速度 | < 50ms/note | 性能基准 |
| 索引构建时间 | < 5min (10k notes) | 性能测试 |
| 内存占用 | < 500MB | 资源监控 |

## 依赖项

- [ ] 嵌入模型选择和集成
- [ ] 向量存储方案确定
- [ ] 搜索算法实现
- [ ] UI 组件开发

## 风险和缓解

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| 嵌入质量差 | 搜索不准 | 多模型对比测试 |
| 向量库性能 | 搜索慢 | 基准测试 + 优化 |
| 内存占用高 | OOM | 流式处理 + 分批 |
| 索引构建慢 | 体验差 | 增量更新 + 后台任务 |

## 相关 Issue

- #XXX: 嵌入模型集成
- #XXX: 向量存储优化
- #XXX: 搜索 UI 增强

## 标签

`feature` `P0` `search` `ai` `performance`

## 里程碑

Phase 2: 智能搜索 (Week 4-7)
