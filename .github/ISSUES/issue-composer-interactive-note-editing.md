## 竞品参考

Obsidian Copilot Plus 提供了 **Composer** 功能：用户可以在聊天中输入 `@composer` 进入编辑模式，随后通过自然语言指令修改笔记——"高亮关键人物名"、"添加相关标签"、"重写这段为更正式的语气"。Composer 生成修改后的笔记版本，用户可通过 Accept/Reject/Revert 操作管理变更。同时支持 Canvas 编辑。

Obsidian Copilot v4 进一步强化了这点：agent 的所有写操作（创建、重命名、修改、删除）都先作为"待审变更"展示 diff 预览，用户批准后才写入文件。

SystemSculpt 也有内置的 review-before-write（写入前审查）机制。

VaultPilot 已有 WriteApprovalDialog（#1453）用于 agent 写操作的审批，但缺少以用户引导为主的、对话式的笔记编辑体验。

## 差距分析

**VaultPilot 当前状态**：VaultPilot 的 Agent Mode 可以自主执行笔记操作（搜索、读取、保存、列出目录）。用户可以通过对话让 AI"帮我写一个刚才讨论的总结"，但：
1. AI 只能进行"全写"（创建新笔记或完全覆盖现有笔记）
2. 无法进行精细的局部编辑（"高亮第三段"、"把日期改成明天"）
3. 没有交互式编辑流程——用户提出修改，AI 展示 diff，用户确认/驳回
4. WriteApprovalDialog 目前只支持 agent 自主发起的操作，不支持用户主动引导的编辑

**竞品做法**：Obsidian Copilot 的 Composer 是一个完整的交互式编辑循环：用户输入 `@composer` + 编辑指令 → AI 分析指令 → AI 生成修改后的笔记版本 → 以 diff 形式展示 → 用户 Accept/Reject/Revert → 如果 Reject，用户可反馈修改意见 → AI 重新生成。

**差距**：缺少对话式的笔记编辑工作流。用户不能"边讨论边改笔记"，而是回到"用户写→AI 写新笔记"的批处理模式。

## 建议方案

### 后端（Rust）新增 Composer 工作流

1. **新增 `composer_edit` tool**：
   - 输入：`note_path`, `edit_instructions`（自然语言编辑指令）
   - 处理流程：
     a. 读取当前笔记内容
     b. 将内容 + 编辑指令发给 LLM
     c. LLM 返回编辑后的完整内容
     d. 生成 unified diff（对比原始内容与编辑后内容）
     e. 返回 `{original_preview, edited_preview, diff}` 给前端
   - **不写入文件**——只返回预览

2. **新增 `composer_apply` / `composer_reject` 端点**：
   - `composer_apply(path)`：确认写入已审批的修改
   - `composer_reject(path)`：驳回修改，清理暂存
   - 复用 WriteApprovalDialog 的部分逻辑

### 前端（WinUI + Mobile）

1. **编辑模式入口**：
   - 在 chat input 中输入 `/edit` 或 `@edit` 触发编辑器模式
   - 或从笔记操作菜单中选择"用 AI 编辑"

2. **交互式编辑 UI**：
   - 展示 side-by-side diff（原文 vs 修改后）
   - 绿色高亮新增行，红色高亮删除行
   - Accept（✓）/ Revise（↻）/ Reject（✗）三按钮
   - Revise 时打开新的编辑指令输入框

3. **与现有 WriteApprovalDialog 的关系**：
   - Composer 是用户主动引导的编辑（用户发起、用户确认）
   - WriteApprovalDialog 是 agent 自主操作的审批（agent 发起、用户审批）
   - 两者可以共享 diff 渲染组件

### 协议层
- JSON-RPC 方法：`composer_preview(path, instructions)` → `{diff, original, edited}`
- JSON-RPC 方法：`composer_apply(path)` / `composer_discard(path)`

## 优先级
**P2** — 重要功能，但不阻塞现有用户使用。

**理由**：这是从"AI 聊天助手"到"AI 协作编辑器"的关键跨越。Obsidian Copilot 已经证明 Composer 是 Plus 用户付费的核心动力之一。VaultPilot 已有 WriteApprovalDialog 基础，实现成本可控。

## 预期影响
- 用户编辑笔记效率提升 3-5 倍（从手动编辑 → 对话式编辑）
- 与 Agent Mode 形成互补：Agent 负责"自主做"，Composer 负责"一起做"
- 产品差异化：竞争对手（Notion AI、Mem）都没有如此精细的本地笔记编辑能力
- 工程成本：后端 ~3 天（diff 生成复用已有代码），WinUI ~3 天，Mobile ~4 天，测试 ~2 天
