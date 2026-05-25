# Codex Quota Widget

一个 Windows 托盘小工具，用来显示 ChatGPT Codex 额度余量：

- 5 小时使用限额剩余百分比和重置时间。
- 每周使用限额剩余百分比和重置日期。
- 托盘图标直接显示 5 小时额度余量。
- 支持手动刷新、定时自动刷新、缓存兜底、连接诊断和日志诊断。

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
app/src-tauri/target/release/bundle/
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

应用会读取本机 Edge 里的 `chatgpt.com` 登录态，或使用设置中临时粘贴的 Cookie Header 来请求：

```text
https://chatgpt.com/backend-api/wham/usage
```

隐私边界：

- 不保存 ChatGPT 密码。
- 临时 Cookie Header 只保存在本次程序内存中，不写入磁盘。
- 日志会脱敏 Cookie、Authorization、Bearer 等敏感字段。
- 本地缓存只保存额度百分比、重置时间、刷新状态和短错误摘要。

本地抓包、DevTools 导出、日志、构建产物和运行态目录已通过 `.gitignore` 排除，不应提交到 Git。

## 常见问题

### 显示网络失败

打开设置里的连接诊断，确认系统代理和 ChatGPT 接口是否可达。如果系统代理指向 `127.0.0.1`，请先确认代理程序正在运行。

### 显示需要登录态

在设置里点击“打开 ChatGPT 页面”，确认 Edge 中已经登录，然后从 analytics 页面的 `wham/usage` 请求复制完整 Cookie Header。

### Edge Cookie 被锁定

Edge 正在运行时可能锁定 Cookie 数据库。可以关闭 Edge 后刷新，或在设置里临时粘贴 Cookie Header。
