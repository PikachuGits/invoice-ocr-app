# 发票 OCR 识别

基于 **Tauri 2 + React** 的发票识别桌面应用：调用 PaddleOCR API 识别发票图片/PDF，识别结果入库，支持列表筛选、批量删除与 Excel 导出。

- 仓库：https://github.com/PikachuGits/invoice-ocr-app
- 支持平台：macOS、Windows、Linux（后端兼容，打包产物按需构建）

## 功能一览

- 单张/批量识别发票（jpg / jpeg / png / bmp / webp / pdf）
- 识别结果持久化（SQLite），识别失败的记录保留并支持重新识别
- 列表：状态 Tab 切换、日期筛选、分页（每页 10/20/50/100）、跨页多选、批量删除、Excel 导出
- Excel 导出：单 sheet 或多 sheet（每张发票一个 sheet）、发票块浅绿背景区分、动态商品明细列
- 设置：API 地址 / Token / 每页条数，独立分栏保存

## 技术栈

| 层 | 技术 |
| --- | --- |
| 桌面壳 | Tauri 2 (Rust) |
| 前端 | React 19 + Vite + TypeScript |
| UI | MUI (Tabs / DatePicker) + 自研 CSS |
| 数据 | SQLite（`rusqlite`） |
| 导出 | rust_xlsxwriter |

## 目录结构

```
├── .github/workflows/release-windows.yml   # Windows 打包 CI
├── src/                                    # 前端
│   ├── components/  Header / RecognizeModal
│   └── pages/       InvoiceListPage / DetailPage / SettingsPage
├── src-tauri/                              # 后端
│   ├── src/
│   │   ├── commands.rs       # Tauri 命令（识别/列表/删除/配置）
│   │   ├── db.rs             # SQLite 建表与查询
│   │   ├── exporter.rs       # Excel 导出
│   │   ├── pdf.rs            # PDF 转图片
│   │   └── invoice_extractor.rs
│   └── tauri.conf.json       # 应用配置（名称/版本/打包）
└── package.json
```

## 环境要求

| 工具 | 版本 |
| --- | --- |
| Node.js | >= 20（推荐 22） |
| pnpm | 10.x |
| Rust | stable（rustup） |
| Xcode Command Line Tools | macOS 打包必需 |

安装 Rust：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

macOS 还需确认：

```bash
xcode-select --install   # 已安装则提示 "already installed"
```

## 本地开发（macOS）

```bash
# 1. 安装前端依赖
pnpm install

# 2. 启动开发模式（热更新，会编译 Rust 后端）
pnpm tauri dev
```

首次编译 Rust 依赖较慢（几分钟），之后增量编译很快。

## 本地打包 macOS

```bash
pnpm tauri build
```

产物目录：`src-tauri/target/release/bundle/`

| 文件 | 说明 |
| --- | --- |
| `dmg/invoice-ocr-app_0.3.0_aarch64.dmg` | 安装包（推荐分发给普通用户） |
| `macos/invoice-ocr-app.app` | 未压缩的 .app 程序 |

> 注意：Tauri 不支持跨平台打包。macOS 上只能打 macOS 包，Windows 安装包需在 Windows 环境或 CI 中构建（见下）。

## 打包 Windows 安装包（GitHub Actions）

项目已配置 CI（`.github/workflows/release-windows.yml`），在 **GitHub 的 Windows 服务器**上自动打包，无需本地 Windows 环境。

### 方式一：打 tag 触发（推荐）

版本号与 `src-tauri/tauri.conf.json` 的 `version` 保持一致：

```bash
# 假设当前版本 0.3.0
git tag v0.3.0
git push origin v0.3.0
```

推送后自动触发打包，约 10 分钟完成。

### 方式二：手动触发

仓库 → **Actions** → **Release Windows** → **Run workflow** → 点击运行。

### 下载产物

1. 打包完成后进入仓库 **Releases** 页
2. 找到对应版本的 **draft（草稿）Release**
3. 检查附件无误后点击 **Publish release**（发布后才会公开）

产物文件：

| 文件 | 说明 |
| --- | --- |
| `invoice-ocr-app_0.3.0_x64-setup.exe` | NSIS 安装包（推荐，双击安装） |
| `invoice-ocr-app_0.3.0_x64_en-US.msi` | MSI 安装包 |

### 离线安装说明（针对无外网/网络受限的机器）

1. **pdfium 已内置，不再联网下载**：`src-tauri/resources/pdfium.dll` 会随安装包分发
   （配置见 `tauri.windows.conf.json` 的 `bundle.resources`）。应用启动后优先加载安装目录下的
   `pdfium.dll`，不会再触发 pdfium-auto 的 GitHub 下载（此前在国内网络下容易超时）。
   - 更新 pdfium 版本：运行 `scripts/download-pdfium.ps1`（Windows）或
     `scripts/download-pdfium.sh`（macOS/Linux），并把新文件提交到仓库。
2. **WebView2 使用离线安装器**：`tauri.windows.conf.json` 中
   `webviewInstallMode.type = "offlineInstaller"`。打包时 CI 会自动下载 WebView2 离线安装器
   （约 127MB）嵌入安装包，目标机器**无需联网**也能装好 WebView2，不再出现“未安装”提示。
   - 代价：安装包体积增加约 127MB；打包机需要能访问微软 CDN（GitHub Actions 自带，无需配置）。

### 费用说明

- 公开仓库：GitHub Actions 完全免费
- 私有仓库：每月免费 2000 分钟，Windows runner 按 2 倍计费（约 1000 分钟/月），单次打包约 10 分钟，远在额度内

## CI 文件说明（release-windows.yml）

文件位置：`.github/workflows/release-windows.yml`，GitHub Actions 会自动识别该目录下的 workflow。

### 结构规范

```yaml
name: Release Windows          # 工作流名称（Actions 页显示）
on:                            # 触发条件
  push:                        # 1. 推送 v* 开头的 tag 时触发
    tags: ["v*"]
  workflow_dispatch:           # 2. 也支持 Actions 页手动触发

permissions:                   # 最小权限声明
  contents: write              #   允许创建 Release/上传产物

jobs:                          # 任务（可多个，并行执行）
  build-windows:               # 任务名
    runs-on: windows-latest    # 运行环境（ubuntu/macos/windows-latest 三选一）
    steps:                     # 步骤列表，按顺序执行
      - uses: actions/checkout@v4            # 拉取代码
      - uses: pnpm/action-setup@v4           # 安装 pnpm（with.version 指定版本）
      - uses: actions/setup-node@v4          # 安装 Node
      - uses: dtolnay/rust-toolchain@stable  # 安装 Rust
      - run: pnpm install --frozen-lockfile  # 命令行步骤
      - uses: tauri-apps/tauri-action@v0     # 打包 + 创建 Release
```

### 语法规范要点

| 要点 | 说明 |
| --- | --- |
| 缩进 | 必须用**空格**（2 格），不能用 Tab |
| 键值 | `键: 值`，冒号后要有空格 |
| 列表 | 以 `- ` 开头（如 steps 每一项） |
| 触发条件 | `tags: ["v*"]` 中 `v*` 是通配符，匹配 `v0.3.0`、`v1.2.3` 等 |
| 版本一致性 | tag 中的版本号必须与 `src-tauri/tauri.conf.json` 的 `version` 一致 |
| Token | `secrets.GITHUB_TOKEN` 由 GitHub 自动提供，无需手动配置 |
| 字符串 | 含中文/空格的值（如 releaseName）用双引号包裹 |
| 环境选择 | `runs-on` 三选一：`ubuntu-latest` / `macos-latest` / `windows-latest`；Windows 打 Windows 包，不能跨平台 |

### 修改打包行为

- 想同时出 macOS 包：复制该文件，把 `runs-on` 改为 `macos-latest`（但你自己是 mac，本地 `pnpm tauri build` 即可，通常不需要）
- 想改产物类型：修改 `tauri.conf.json` 的 `bundle.targets`（当前为 `all`，Windows 下会产出 exe + msi）
- 想关闭草稿直接发布：把 `releaseDraft: true` 改为 `false`

## 发新版本流程（完整步骤）

```bash
# 1. 修改版本号（三处保持一致）
#    - src-tauri/tauri.conf.json 的 "version"
#    - 如需可在 src/pages/SettingsPage.tsx 的"关于"里同步版本号

# 2. 提交代码
git add -A
git commit -m "feat: xxx"

# 3. 打 tag 并推送（tag 名 = v + 新版本号，如 v0.3.0）
git tag v0.3.0
git push origin main
git push origin v0.3.0

# 4. 等待 CI 打包完成（仓库 Actions 页看进度）
# 5. 到 Releases 页发布草稿
```

## 首次使用配置

1. 打开应用 → 右上角 **设置**（⚙ 图标）
2. **API 配置**：填写 PaddleOCR API 地址与 Token，保存
3. **分页配置**：选择列表每页显示条数，保存
4. 返回首页点击 **识别发票**，选择图片/PDF 文件即可

## 数据存储

- 数据库：`~/Library/Application Support/invoice-ocr-app/invoices.db`（macOS），Windows 在 `%APPDATA%\invoice-ocr-app\`
- 备份：直接复制该 `.db` 文件即可；识别附件文件也保存在同一目录

## 安全提示（Token 管理）

> ⚠️ **重要**：早期版本曾将 API Token 硬编码并推送到公开仓库（`src-tauri/config.json`、`DEFAULT_TOKEN` 常量，位于 git 历史中）。**请立即到 PaddleOCR 平台更换一个新的 Token**，旧 Token 已视为泄露。代码中的硬编码 Token 已全部移除。

Token 的配置优先级（从高到低）：

| 优先级 | 位置 | 说明 |
| --- | --- | --- |
| 1 | 应用内「设置 → API 配置」 | 保存在本地 SQLite 数据库（推荐） |
| 2 | `.env` 文件 | `PADDLEOCR_TOKEN=xxx`（见 `.env.example`） |
| 3 | `config.json` | 可执行文件同目录（本地使用，**不提交 git**） |

规则：

- `.env` 与 `src-tauri/config.json` 已加入 `.gitignore`，**永远不要提交**
- 未配置 Token 时识别会直接报错提示（不再有默认 Token 兜底）
- 提交代码前检查：`git status` 中不应出现 `.env` / `config.json`；也可用 `git diff --cached | grep -i token` 复查
- 更换 Token 后，数据库中旧 Token 需手动在设置页更新

## 常用开发命令

| 命令 | 作用 |
| --- | --- |
| `pnpm dev` | 仅启动前端（配合 `pnpm tauri dev` 使用） |
| `pnpm tauri dev` | 本地开发运行 |
| `pnpm tauri build` | 本地打包当前平台 |
| `pnpm build` | 仅构建前端产物 |
| `cd src-tauri && cargo test` | 运行 Rust 单元测试 |
