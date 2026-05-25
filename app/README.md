# App

Codex Quota Widget 的 Tauri 应用目录。

功能包括：

- Windows 托盘常驻。
- 详情窗口显示 5 小时和每周额度。
- 托盘图标显示 5 小时额度余量。
- 手动刷新和定时自动刷新。
- Edge Cookie 自动读取、临时 Cookie Header、连接诊断和本地日志。

## Development

```powershell
npm install
npm run tauri dev
```

## Build

```powershell
npm run build
npm run tauri -- build
```

Release artifacts are written under `src-tauri/target/release/bundle/`.

## Data Source

默认请求 ChatGPT Codex 使用量接口：

```text
https://chatgpt.com/backend-api/wham/usage
```

开发解析器时也可以设置 `CODEX_QUOTA_ANALYTICS_TEXT`，让应用使用本地样本文本而不是 live 请求。
