# 随包视频工具

Travel Gallery 通过独立的 FFmpeg/FFprobe 子进程读取视频元信息、生成海报，不链接 FFmpeg 库。Release 只使用主程序旁随包工具；应用运行时不安装工具、不下载模型。WebView 内嵌播放由系统解码器负责。

## 构建

```sh
pnpm media:tools:build
node scripts/smoke-media-tools.mjs
```

版本、源码 URL 和 SHA-256 固定在 `third-party/media-tools/profile.json`；构建前校验源码归档，拒绝跨平台预编译二进制。输出为 Tauri sidecar 和 `src-tauri/resources/media-tools/` 下的对应源码、许可及构建材料。

macOS 在目标机器原生构建 arm64 或 x64，最低系统版本 13.0；x64 构建需要 nasm。

Windows x64 使用 MSYS2 UCRT64，所需软件包：

```text
make diffutils curl tar
mingw-w64-ucrt-x86_64-gcc
mingw-w64-ucrt-x86_64-nasm
mingw-w64-ucrt-x86_64-pkgconf
mingw-w64-ucrt-x86_64-zlib
```

`TRAVEL_GALLERY_MSYS2_ROOT` 可指定 MSYS2 根目录。FFmpeg 使用 GCC 编译，其 sidecar 文件名按 Rust 的 `x86_64-pc-windows-msvc` 桌面目标命名。构建脚本检查 PE 导入表，拒绝非系统运行库依赖。

工具只启用本地 `file` 协议，冒烟检查覆盖空 PATH、自建短视频、元数据、JPEG 海报及网络协议拒绝，不修改原始素材。源码缓存位于用户缓存目录 `travel-gallery/media-tools`，生成文件不纳入 Git。

## 许可材料

FFmpeg/FFprobe 使用 LGPL-2.1-or-later。固定源码、构建配置、许可和来源材料与应用一起分发。`pnpm notices` 生成应用内第三方告知及 `third-party/dependency-inventory.json`，`pnpm notices:check --self-test` 可核对现有材料。实际分发仍需核对适用的第三方授权及专利条件。

参考：[FFmpeg Windows 构建](https://ffmpeg.org/platform.html#Windows)、[MSYS2 CI](https://www.msys2.org/docs/ci/)。
