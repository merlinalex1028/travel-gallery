# 旅图 · Travel Gallery

纯离线的旅行照片与视频管理桌面应用。引用原文件，使用本地 SQLite 保存索引、评分、标签和相册，支持挑片、组合筛选、查重、复制导出和资料库恢复。

## 安装

从 [GitHub Releases](https://github.com/merlinalex1028/travel-gallery/releases) 下载：

- macOS Apple Silicon：arm64 DMG，要求 macOS 13 或更新版本。打开后将 App 拖到“应用程序”。
- Windows 10/11 x64：运行 setup.exe，包含离线 WebView2 安装器。

当前安装包没有受系统信任的发布者签名：macOS 使用 ad-hoc 完整性签名且未公证，Windows 未做 Authenticode 签名。首次启动可能出现安全提示；确认下载来源和 SHA-256 后，macOS 可在“系统设置 → 隐私与安全性”中选择“仍要打开”，Windows 若显示 SmartScreen 提示可选择“更多信息 → 仍要运行”。受组织策略管理的设备可能无法手动放行。无需关闭系统的全局安全功能。

将下载文件的 SHA-256 与 Release 附件 `SHA256SUMS.txt` 核对：

```sh
shasum -a 256 Travel-Gallery-*-macOS-arm64.dmg
```

Windows PowerShell 使用 `Get-FileHash .\Travel-Gallery-*-Windows-x64-setup.exe -Algorithm SHA256`。

## 使用

1. 添加照片目录，后台索引和生成缩略图；原图保留在原处。
2. 在图库筛选、排序、多选，进入挑片工作台评分和标记。
3. 管理标签、相册及旅行故事；人工复核精确重复、视觉相似和连拍组。
4. 导出前查看数量、空间、冲突和缺失预演；可选附带评分、标签和备注 XMP。
5. 在设置中备份/恢复资料库。备份不包含源照片、视频或离线模型包，请独立保管。

挑片快捷键：`P` 保留、`X` 淘汰、`U` 待定、`0–5` 评分、`F` 收藏、`←/→` 翻图、空格适应/原图、`Tab` 详情、`⌘/Ctrl+Z` 撤销。

本地 AI 和闭眼/细节提示默认关闭，需手动导入独立模型包。应用不自动下载模型、删除原图或按 AI 结果自动淘汰。地图使用随包的 Natural Earth 概览，没有在线地图请求。视频工具说明见 [MEDIA-TOOLS.md](docs/MEDIA-TOOLS.md)，第三方许可可在应用设置中离线查看。

## 开发

需要 Node.js 22、pnpm 10、Rust stable 和 [Tauri 平台环境](https://v2.tauri.app/start/prerequisites/)。

```sh
pnpm install --frozen-lockfile
pnpm desktop   # 独立开发资料目录和编译目录
pnpm dev       # 浏览器模拟数据
pnpm test
pnpm build
pnpm test:rust
```

开发版标识为 `com.travelgallery.development`；正式版为 `com.travelgallery.desktop`。开发与验收必须使用隔离资料库和自有临时素材，不覆盖正式数据，不修改用户原图。

Vue 3 / TypeScript / Pinia / Element Plus 前端，Rust / Tauri 2 / SQLite 后端。测试 fixture 位于 `tests/fixtures/`；依赖来源和许可材料位于 `third-party/`。临时测试素材应在结束时清理。

## 构建与发布

```sh
pnpm desktop:build           # macOS App / Windows NSIS
pnpm desktop:dmg             # 本机 DMG
pnpm desktop:package:mac     # 本机 ad-hoc 审阅 ZIP
pnpm desktop:package:windows # Windows 安装包
```

推送版本标签自动构建 macOS arm64 DMG 和 Windows x64 安装包，默认无需签名凭据。标签规则、产物和可选的正式签名配置见 [发布指南](docs/RELEASING.md)。

沟通记录、设计草稿、交接文档、测试收据和本机交付元数据不纳入版本控制。
