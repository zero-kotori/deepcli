# deepcli 交接摘要

本文只保留当前仍有用的交接信息。完整当前上下文见 `docs/ai/CONTEXT.md`；架构和模块边界见 `docs/ARCHITECTURE.md`、`docs/HARNESS.md` 和 `docs/MODULES/`。

## 当前状态

- 旧的大型命令入口已经拆分到 `src/commands/*.rs`，`src/commands.rs` 主要负责分发和 re-export。
- 旧 fullscreen TUI 已删除，当前 UI 是原生终端聊天路径：`src/ui.rs`、`src/ui/native_terminal.rs`、`src/ui/resume_picker.rs`。
- scorecard、round、benchmark、recipes 和 opportunities 是本地健康、证据和工作流报告，不再作为开放式产品目标文档维护。
- 旧文档中的 `recipes sota` 仍是兼容主题名；新增文档应使用“产品证据工作流”或“benchmark 工作流”描述它。

## 继续工作时优先读取

1. `README.md`
2. `docs/ai/CONTEXT.md`
3. `docs/ARCHITECTURE.md`
4. `docs/HARNESS.md`
5. `docs/COMMANDS.md`
6. 相关 `docs/MODULES/*.md`
7. 待修改源码和测试

## 提交前检查

文档-only 改动至少运行：

```bash
git diff --check
./scripts/deepcli privacy --no-history --json
```

涉及命令文档或模块边界时，按 `docs/HARNESS.md` 的验证章节补跑对应契约测试。
