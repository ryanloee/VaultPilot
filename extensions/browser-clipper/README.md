# VaultPilot Browser Clipper Extension

一键将网页内容保存为 VaultPilot 笔记。

## 安装

### Chrome / Edge (Chromium)

1. 打开 `chrome://extensions`
2. 开启「开发者模式」
3. 点击「加载已解压的扩展程序」
4. 选择本项目目录：`extensions/browser-clipper/`

### Firefox

Firefox 版本需要调整 manifest（MV3 部分 API 在 Firefox 中实现不同），正在适配中。

## 使用方法

### 方式一：点击扩展图标
1. 在任意网页点击 VaultPilot 图标
2. 扩展自动提取页面正文内容
3. 点击「保存当前页面」
4. 页面内容通过 VaultPilot HTTP Bridge 保存为笔记

### 方式二：右键菜单
- 右键点击页面 →「保存页面到 VaultPilot」
- 选中文本后右键 →「保存选中内容到 VaultPilot」

### 方式三：快捷键（需自行配置）
在 `chrome://extensions/shortcuts` 中为 VaultPilot Clipper 配置快捷键。

## 前置要求

VaultPilot 的 HTTP Bridge 必须处于运行状态：

```bash
# 启动 HTTP Bridge（默认端口 10101）
cd vaultpilot
./target/release/vaultpilot-cli http-bridge --host 127.0.0.1 --port 10101

# 如果需要认证
./target/release/vaultpilot-cli http-bridge --host 127.0.0.1 --port 10101 --token my-secret-token
```

## 配置

在扩展设置页面（右键图标 →「选项」）中配置：

| 配置项 | 说明 | 默认值 |
|--------|------|--------|
| API 地址 | VaultPilot HTTP Bridge 地址 | `http://127.0.0.1:10101` |
| API Token | 认证令牌（如有） | 空 |
| 默认标签 | 每次保存自动添加的标签 | `clipper` |
| 目标集合 | 笔记自动归入的集合 ID | 空（默认位置） |
| 保存格式 | 简洁模式 / 完整模式 | 简洁 |
| 点击即存 | 图标点击直接保存，跳过弹窗 | 关闭 |

## API

扩展通过 `POST /api/notes` 端点写入笔记，请求格式：

```json
{
  "title": "页面标题",
  "body": "笔记正文（含引用来源的 Markdown）",
  "source": "来源站点名称",
  "sourceUrl": "https://example.com/page",
  "tags": ["clipper", "标签1"],
  "collectionId": ""
}
```

## 技术说明

- Manifest V3，使用 Service Worker
- 正文提取：DOM 克隆 + 清理 + `document.body.textContent`
- 存储：`chrome.storage.sync` 实现设置同步
- 图标：256x256 蓝色圆圈，表示 VaultPilot 品牌色

## 项目结构

```
extensions/browser-clipper/
├── manifest.json        # 扩展清单（MV3）
├── background.js        # Service Worker（处理图标点击、右键菜单、API 通信）
├── content.js           # Content Script（页面内容提取）
├── popup.html           # 弹窗界面
├── popup.js             # 弹窗逻辑
├── options.html         # 设置页面
├── options.js           # 设置逻辑
└── icons/               # 应用图标
    ├── icon16.png
    ├── icon48.png
    └── icon128.png
```
