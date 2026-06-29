# ADR 0002：命令面收束与重复别名删除

## 状态
已接受并执行中。

## 背景
命令面积累了大量重复别名：provider/model（/provider /use /switch /models /providers）、凭证（/auth /key + /credentials 的 template/import-env）、会话（/about /health /history /next）、环境（/env /check /docker /setup）以及一批未文档化的解析别名。同一命令清单被冗余硬编码在 parser、registry、help、wrapper（多份拷贝）、cli 顶层归一化、scorecard 计分、selftest、补全脚本生成、UI 面板和多个测试文件里，导致维护成本高、易漂移。

## 决策
删除重复/低价值别名，只保留规范命令：
- 模型：保留 `/model`。
- 凭证：保留 `/login /logout /apikey` 与 `/credentials`（裸=status、set、remove）。`/credentials set|remove` 与短别名是同一代码路径，无法单独删除。
- 会话：保留 `/session /cleanup /doctor /rename`。
- 环境：保留一步命令 `/install /compiler`，删除 `/env` 命令面；docker/compiler 仍是 `/doctor`/`/diagnose` 的有效 target，Env 内部处理器保留。
- 删除全部未文档化解析别名。

## 影响
- 删除每个家族需同步约 10 个文件；以"编译 + 全测试 + 全仓 grep 残留"验证。
- `docs/COMMANDS.md` 与命令 registry 的一致性由 `tests/mvp_contract.rs::command_docs_match_registry` 守护。
- 命令清单冗余硬编码本身是后续 Stage 2 去硬编码的目标（wrapper 的多份命令列表尤为脆弱）。
