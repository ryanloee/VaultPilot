# VaultPilot

Local knowledge base assistant for recording, searching, and organizing engineering notes.

[中文说明](#中文说明)

## What VaultPilot Can Do

- **Grounded Q&A**: Ask questions in natural language. VaultPilot searches your local notes, uses AI to generate evidence-based answers, and includes source references.
- **Note Management**: Store engineering notes as Markdown files with metadata such as tags, keywords, platform, board, and kernel for easier retrieval later.
- **Knowledge Indexing**: Build a full-text search index for your notes automatically, with support for rebuilding the index when needed.
- **Conversation Memory**: Keep full chat history and summarize long conversations so context remains usable over time.
- **File and Command Operations**: Let AI read local files, list directories, and run command-line operations, then fold the results into answers.
- **Image Support**: Copy related images automatically when importing notes, and attach screenshots in chat.
- **Markdown Import**: Import existing Markdown files into the knowledge base in bulk.

## Data Storage

All data stays on your local machine. Note management and search work offline. AI-powered Q&A requires an API key.

- **Notes**: Stored as Markdown files in your selected vault directory.
- **Indexes and State**: Stored in the local application data directory.

## System Requirements

- Windows 10 version 1809 or later
- For AI features: an Anthropic API key

---

## 中文说明

本地知识库助手，用于记录、检索和整理工程笔记。

## VaultPilot 能做什么

- **智能问答**：用自然语言提问，VaultPilot 会在你的本地笔记库中搜索相关内容，结合 AI 生成有据可依的回答，并附上引用来源。
- **笔记管理**：以 Markdown 文件保存工程笔记，支持标签、关键词、平台、板卡、内核等元数据分类，方便后续检索。
- **知识索引**：自动对笔记建立全文搜索索引，并支持在需要时重建索引以保持搜索结果准确。
- **会话记忆**：保留完整聊天历史，并支持对长对话进行摘要，确保长期使用后仍能追溯上下文。
- **文件与命令操作**：AI 可以读取本地文件、列出目录内容、执行命令行操作，并将结果直接整合到回答中。
- **图片支持**：导入笔记时自动复制关联图片，聊天中也可以附带截图提问。
- **Markdown 导入**：支持批量导入已有的 Markdown 文件到知识库中。

## 数据存储

所有数据都保存在本地机器上。笔记管理和搜索功能无需联网即可使用，AI 问答功能需要配置 API 密钥。

- **笔记文件**：以 Markdown 格式存放在你指定的 Vault 目录中。
- **索引与状态**：保存在本地应用数据目录中。

## 系统要求

- Windows 10 1809 及以上版本
- 使用 AI 功能时需要一个 Anthropic API 密钥
