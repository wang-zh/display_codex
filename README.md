# Codex Quota Widget

一个 Windows 托盘小工具，用来显示 ChatGPT Codex 额度余量：

- 5 小时使用限额剩余百分比和重置时间。
- 每周使用限额剩余百分比和重置日期。
- 托盘图标直接显示 5 小时额度余量。
- 支持手动刷新、定时自动刷新、缓存兜底、连接诊断和日志诊断。
- 优先复用本机 Codex CLI 的 ChatGPT 登录态，减少浏览器 Cookie 依赖。

## 技术栈

- Tauri 2
- Rust
- TypeScript
- Vite

## 项目结构

```text
app/
  src/                 前端界面
  src-tauri/src/       Rust 后端、托盘、取数、解析和诊断
  src-tauri/tests/     Rust 集成测试
  src-tauri/nsis/      Windows 安装器钩子
```

## 使用教程

### 1. 安装

从 GitHub Releases 下载最新 Windows 安装包：

```text
https://github.com/wang-zh/display_codex/releases/latest
```

推荐下载并运行 NSIS 安装包：

```text
Codex.Quota.Widget_*_x64-setup.exe
```

如果是本地构建，安装包位于：

```text
app/src-tauri/target/release/bundle/nsis/*.exe
app/src-tauri/target/release/bundle/msi/*.msi
```

覆盖安装时，安装器会尝试自动结束正在运行的旧进程，然后安装新版本。

### 2. 首次启动

启动后应用会常驻系统托盘：

- 托盘图标会直接显示 5 小时额度余量。
- 鼠标悬停托盘图标会显示简略额度信息。
- 点击托盘图标可以打开或隐藏详情卡片。
- 在托盘菜单中可以执行打开详情、立即刷新、打开设置和退出程序。

应用默认会优先读取本机 Codex CLI 的 ChatGPT 登录态。如果不可用，会继续尝试设置里临时粘贴的 Cookie Header，然后再尝试读取 Edge 中的 `chatgpt.com` 登录态。

### 3. 配置 Codex 登录态

推荐先在终端完成 Codex ChatGPT 登录：

```powershell
codex login
codex login status
```

当状态显示为下面内容时，应用可以直接复用本地 Codex 登录态查询额度：

```text
Logged in using ChatGPT
```

如果本机没有安装 Codex CLI、登录态过期，或使用的是 API Key 登录模式，应用会自动降级到临时 Cookie Header 和 Edge Cookie。

### 4. 配置 Cookie Header

1. 在 Edge 中登录 ChatGPT。
2. 打开额度页面：

```text
https://chatgpt.com/codex/cloud/settings/analytics#usage
```

3. 使用F12打开 DevTools，进入 Network 面板。
4. 找到这个请求：

```text
https://chatgpt.com/backend-api/wham/usage
```

5. 复制 Request Headers 里的完整 `Cookie` 值。
6. 回到应用设置页，粘贴到“临时 Cookie Header”，点击应用并刷新。

临时 Cookie Header 只保存在本次程序内存中，不会写入磁盘。重启后如果自动读取 Codex 登录态和 Edge 登录态都失败，需要重新粘贴。

### 5. 刷新与显示

- 点击详情卡片右上角“刷新”可以手动刷新。
- 设置页可以调整自动刷新间隔，默认每 5 分钟刷新一次。
- 5 小时额度的重置时间显示为具体时间。
- 每周额度的重置时间显示为具体日期。
- 如果当前网络失败但本地有缓存，界面会继续显示上一次可用数据，并标记数据来源为缓存。

### 6. 诊断与日志

设置页提供连接诊断，用来检查系统代理、本地代理端口、ChatGPT session、analytics 页面和 `wham/usage` 接口是否可达。

日志文件位于：

```text
C:\Users\<你的用户名>\AppData\Local\com.codex.quota.widget\codex-quota.log
```

日志会脱敏 Cookie、Authorization、Bearer 等敏感字段。提交 issue 或排查问题时，可以优先查看日志里的最近一次刷新记录和错误摘要。

## 本地开发

```powershell
cd app
npm install
npm run tauri dev
```

## 构建安装包

```powershell
cd app
npm run tauri -- build
```

生成文件位于：

```text
app/src-tauri/target/release/bundle/nsis/*.exe
app/src-tauri/target/release/bundle/msi/*.msi
```

## 发布 GitHub Release

推荐使用 GitHub CLI 发布安装包。第一次使用前先登录：

```powershell
gh auth login
```

构建并确认安装包存在：

```powershell
cd app
npm run tauri -- build
cd ..
```

发布前确认代码已经提交并推送，版本号和 Release 标签一致。发布当前版本，例如 `v0.2.0`：

```powershell
gh release create v0.2.0 `
  "app/src-tauri/target/release/bundle/nsis/Codex Quota Widget_0.2.0_x64-setup.exe" `
  "app/src-tauri/target/release/bundle/msi/Codex Quota Widget_0.2.0_x64_en-US.msi" `
  --repo wang-zh/display_codex `
  --title "Codex Quota Widget v0.2.0" `
  --notes "Windows 托盘版 Codex 额度显示工具。"
```

如果不想使用命令行，也可以在 GitHub 网页中进入：

```text
https://github.com/wang-zh/display_codex/releases/new
```

## 验证命令

```powershell
cd app/src-tauri
cargo fmt --check
cargo test

cd ../
npm run build
```

## 数据与隐私

应用会优先读取本机 Codex CLI 的 ChatGPT 登录态，必要时读取本机 Edge 里的 `chatgpt.com` 登录态，或使用设置中临时粘贴的 Cookie Header 来请求：

```text
https://chatgpt.com/backend-api/wham/usage
```

隐私边界：

- 不保存 ChatGPT 密码。
- 不复制、不写出本机 Codex 的 `auth.json` 内容。
- Codex access token 只用于本次请求，不展示、不写入日志。
- 临时 Cookie Header 只保存在本次程序内存中，不写入磁盘。
- 日志会脱敏 Cookie、Authorization、Bearer 等敏感字段。
- 本地缓存只保存额度百分比、重置时间、刷新状态和短错误摘要。

本地抓包、DevTools 导出、日志、构建产物和运行态目录已通过 `.gitignore` 排除，不应提交到 Git。

## 常见问题

### 显示网络失败

打开设置里的连接诊断，确认系统代理和 ChatGPT 接口是否可达。如果系统代理指向 `127.0.0.1`，请先确认代理程序正在运行。

### 显示需要登录态

先确认本机 Codex CLI 是 ChatGPT 登录模式：

```powershell
codex login status
```

如果不是 `Logged in using ChatGPT`，请运行 `codex login` 重新登录。仍失败时，在设置里点击“打开 ChatGPT 页面”，确认 Edge 中已经登录，然后从 analytics 页面的 `wham/usage` 请求复制完整 Cookie Header。

### Edge Cookie 被锁定

Edge 正在运行时可能锁定 Cookie 数据库。可以关闭 Edge 后刷新，或在设置里临时粘贴 Cookie Header。
