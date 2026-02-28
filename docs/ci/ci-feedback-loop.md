# CI 失败闭环（M4）

日期：2026-02-27  
执行者：Codex

## 目标

在“单次受控构建”后，自动回收失败日志，并同步更新：

- `CI_FAILURES.md`（根因 + 关键日志片段）
- `NEXT_ACTION.md`（唯一下一步修复动作）
- `TASK_BOARD.md`（Backlog 自动追加 CI 修复项）

## 入口脚本

- 主脚本：`scripts/ci/collect_ci_failure.py`
- 一键封装：`scripts/ci/refresh_ci_feedback.sh`

## 用法

1. 指定 run_id 回收失败：

```bash
bash scripts/ci/refresh_ci_feedback.sh \
  --repo sikuai2333/zeroclaw \
  --run-id 22440621743
```

> 默认会同时更新：`CI_FAILURES.md`、`NEXT_ACTION.md`、`TASK_BOARD.md`。

2. 自动选择最近失败 run（可选按 workflow 名过滤）：

```bash
bash scripts/ci/refresh_ci_feedback.sh \
  --repo sikuai2333/zeroclaw \
  --workflow "Pub Ubuntu Build"
```

3. 指定输出路径（可选）：

```bash
bash scripts/ci/refresh_ci_feedback.sh \
  --repo sikuai2333/zeroclaw \
  --run-id 22440621743 \
  --ci-failures CI_FAILURES.md \
  --next-action NEXT_ACTION.md \
  --task-board TASK_BOARD.md
```

## 关键改进

1. 修复 `gh api --silent` 导致 JSON body 为空的问题。  
2. 增加空响应容错与 fallback。  
3. 支持同时写入 `NEXT_ACTION.md`（`--write-next-action`）。
4. 支持自动写入 `TASK_BOARD.md` Backlog（`--write-task-board` + `--task-board`）。

## 约束

- 脚本只做“结果回收与回写”，不会触发 workflow。  
- workflow 触发仍走 `feature gate + workflow_dispatch`，保持有节制。
