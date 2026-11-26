# 自动发布桌面安装包

[GitHub Actions 工作流](../.github/workflows/release.yml)在推送 `vMAJOR.MINOR.PATCH` 标签时构建 macOS arm64 DMG 和 Windows x64 NSIS 安装程序，也支持 GitHub Release 的 `published` 事件和手动输入已有标签。

## 默认无需签名凭据

当前默认 `RELEASE_SIGNING=unsigned`：

- macOS App 使用 ad-hoc 完整性签名，不包含 Developer ID 身份，不提交 Apple 公证。
- Windows 安装程序不做 Authenticode 签名，继续包含离线 WebView2 安装器。
- 两端安装包验证完成后一起上传，并附构建版本、源码 commit 和 `SHA256SUMS.txt`。Release 说明与 JSON 构建记录明确标注签名状态。
- macOS Gatekeeper 和 Windows SmartScreen 可能要求手动确认，详见 [README 安装说明](../README.md#安装)。

无需配置 Secrets。仓库变量 `RELEASE_SIGNING` 未设置或设置为 `unsigned` 即可运行。签名 job 使用 `release-signing` Environment；可在 Settings → Environments 管理标签权限。无需人工审批时不要为该 Environment 添加 required reviewers。

## 发布和重试

```sh
git tag v0.1.0-rc.2
git push origin v0.1.0-rc.2
```

只为已经包含工作流的提交创建标签。标签也可用 `v0.1.0` 等稳定版本；预发布后缀生成 prerelease。版本数字不使用前导零，三个字段不超过 `65535`，不接受 build metadata。

工作流在一次性 runner checkout 中同步 package.json、Tauri 配置、Cargo.toml 和 Cargo.lock 的项目版本，不回写仓库或移动标签。FFmpeg/FFprobe 从固定校验值的源码在目标平台构建；安装包不包含本机资料库、测试照片和模型权重。

产物示例：

```text
Travel-Gallery-0.1.0-rc.2-macOS-arm64.dmg
Travel-Gallery-0.1.0-rc.2-Windows-x64-setup.exe
macos-arm64.json
windows-x64.json
SHA256SUMS.txt
RELEASE-NOTES.md
```

两个平台都成功后才创建草稿、上传并发布；已有可变 Release 可补齐同名附件。启用 immutable releases 的仓库应推送新标签，由工作流上传草稿附件后发布；已发布的不可变版本不能覆盖。Actions 中转附件仅保留 1 天。

重试：Actions → Release desktop installers → Run workflow，填写已有标签。修改工作流后使用新版本标签，已有标签始终指向原提交。

## GitHub Pages 项目官网

官网源码位于 `site/`，通过独立工作流 `.github/workflows/pages.yml` 发布到：

`https://merlinalex1028.github.io/travel-gallery/`

在仓库 Settings → Pages 将 Source 设为 **GitHub Actions**。工作流使用 `github-pages` Environment；只有部署任务需要 `pages: write` 与 `id-token: write`。无需 Node 依赖安装，也不构建或启动桌面应用。

以下情况会更新官网：

- `main` 上的官网源码、公共品牌资源或构建脚本变化。
- 发布、编辑或调整 GitHub Release 类型，自动更新下载信息。
- `Release desktop installers` 工作流成功结束；覆盖使用 `GITHUB_TOKEN` 自动发布、不触发后续 `release` 工作流的情况。
- 在 Actions → Deploy project website 中手动运行。

构建时从 Releases 解析包含两个平台安装包的最新正式版本；尚无正式版本时使用最新预发布，并在页面明确标注。未加载下载信息时，链接回退到 Releases 页面。官网不会在访客浏览时调用 GitHub API，也没有统计脚本或第三方字体服务。

本地预览：

```sh
pnpm site:build
pnpm site:preview
# 打开 http://127.0.0.1:4173/site/
```

需要核对特定发布数据时，将 GitHub Releases API 返回的数组保存到临时文件，再运行：

```sh
node scripts/build-site.mjs --releases /path/to/temporary-releases.json
```

发布包仅包含静态 HTML、CSS、JavaScript、生成的下载信息、公共 Logo、本地字体及其许可、内置风景插画和公开演示界面图。内部协作记录、用户媒体、资料库、模型和安装包均不打入官网。演示图使用内置示例素材，不包含用户照片。

## 可选的正式签名

准备好以下身份后，将仓库 Actions Variable `RELEASE_SIGNING` 改为 `signed`。此模式缺少任何配置都会报错，不自动降级。

### macOS

需要 Apple Developer Program 的 Developer ID Application 证书，以及 Apple 公证身份。在 `release-signing` Environment 或仓库配置：

| Secret                       | 内容                                                               |
| ---------------------------- | ------------------------------------------------------------------ |
| `APPLE_CERTIFICATE`          | 带私钥、有密码的 Developer ID Application `.p12` 文件的单行 base64 |
| `APPLE_CERTIFICATE_PASSWORD` | `.p12` 导出密码                                                    |
| `APPLE_SIGNING_IDENTITY`     | 完整身份，如 `Developer ID Application: Name (ABCDE12345)`         |
| `APPLE_API_KEY`              | App Store Connect 团队 API Key 的 Key ID                           |
| `APPLE_API_ISSUER`           | 对应 Issuer ID                                                     |
| `APPLE_API_PRIVATE_KEY`      | `.p8` 私钥完整文本，保留换行                                       |

工作流签名、公证 App，随后签名验证、公证外层 DMG，并验证两个票据。临时私钥在结束时删除。首次公证可能等待较久。

### Windows

接入 Microsoft Azure Artifact Signing 的 Public Trust certificate profile，通过 GitHub OIDC 获取短期授权。

| 类型     | 名称                        | 内容                                                           |
| -------- | --------------------------- | -------------------------------------------------------------- |
| Secret   | `AZURE_CLIENT_ID`           | Entra application Client ID                                    |
| Secret   | `AZURE_TENANT_ID`           | Tenant ID                                                      |
| Secret   | `AZURE_SUBSCRIPTION_ID`     | 签名账户订阅 ID                                                |
| Variable | `ARTIFACT_SIGNING_ENDPOINT` | 账户 HTTPS endpoint，例如 `https://eus.codesigning.azure.net/` |
| Variable | `ARTIFACT_SIGNING_ACCOUNT`  | 签名账户名                                                     |
| Variable | `ARTIFACT_SIGNING_PROFILE`  | Public Trust certificate profile 名                            |
| Variable | `WINDOWS_PUBLISHER_NAME`    | 已验证发布者的证书 SimpleName/CN，精确匹配                     |

在 Azure 完成身份验证并创建签名 profile，建立 Entra application/service principal，配置 federated credential：

- Issuer：`https://token.actions.githubusercontent.com`
- Audience：`api://AzureADTokenExchange`
- Subject：`repo:OWNER/REPOSITORY:environment:release-signing`

在 profile 范围赋予 service principal `Artifact Signing Certificate Profile Signer` 角色。使用微软 `ArtifactSigning 0.1.20` 模块，长时间编译结束后才登录；Tauri 在封装时对主程序、辅助程序、卸载程序和安装程序调用签名钩子，并验证发布者和时间戳。

有效签名不保证所有新版本都无 SmartScreen 提示。GitHub 构建证明和自动更新签名也不替代系统代码签名。

参考：[Tauri macOS 签名](https://v2.tauri.app/distribute/sign/macos/)、[Microsoft 签名服务](https://learn.microsoft.com/en-us/azure/artifact-signing/how-to-signing-integrations)、[OIDC 设置](https://github.com/Azure/artifact-signing-action/blob/main/docs/OIDC.md)。

## 本地检查

```sh
node --test scripts/tests/release.test.mjs scripts/tests/media-tools-build.test.mjs
bash -n scripts/verify-macos-release.sh
```

还需使用 actionlint 检查工作流；真实平台构建与安装以 GitHub runner 和目标设备结果为准。
