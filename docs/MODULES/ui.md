# UI 模块

## 职责

当前 UI 只保留原生终端聊天路径，不再包含 fullscreen TUI/Ratatui 实现。

- `src/ui.rs` 是薄入口：注册 `src/ui/native_terminal.rs` 和 `src/ui/resume_picker.rs`，对外导出 `run_basic_repl`、`pick_resume_session` 和 `ResumeSelection`。
- `src/ui/native_terminal.rs` 拥有默认交互聊天：session header、彩色 `user` 输入标签、raw-mode 多行输入编辑、方向键移动、assistant delta 流式输出、provider turn 状态线、折叠工具进度摘要，以及 plan 采访问题的文本选项展示和回答记录。
- `src/ui/resume_picker.rs` 拥有 `--resume-picker` 的原生终端选择循环：打印可恢复 session 列表，支持数字、唯一 session id 前缀、空输入选择第一项和 `q` 取消。

## 边界

- UI 不再进入 alternate screen，不维护 `TuiState`、Ratatui layout、monitor tabs、dialog、command palette、dashboard 或运行中 TUI 本地命令旁路。
- `deepcli` 默认进入 native terminal；`deepcli repl` 仍作为 native terminal 兼容入口。
- `deepcli tui` 和 `--tui` 已移除；CLI 会显式报错并提示使用 `deepcli`。
- UI 层只负责终端展示和输入采集；命令、工具、权限、会话和 provider 行为仍由对应领域模块拥有。

## 测试

- `cargo test ui::native_terminal::tests --lib` 覆盖原生终端输入编辑、提示符、工具进度折叠和 plan 采访问题文本展示。
- `cargo test ui_entrypoint_is_native_terminal_only_boundary --test mvp_contract` 防止旧 TUI 标记回到 `src/ui.rs`。
- `scripts/native-terminal-smoke` 用真实 pty 启动默认入口、发送 `/quit`，并检查原生终端聊天关键输出。

## 文档同步

- UI 入口、resume picker 或 native terminal 行为变化时，同步更新 `docs/MODULES/ui.md`、`docs/ARCHITECTURE.md` 和 `docs/CORE_FEATURES.md`。
- 删除或恢复交互入口时，同步更新 `scripts/deepcli`、completion metadata 和 wrapper/contract 测试。
