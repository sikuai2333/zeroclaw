# CI 门禁与触发策略（M1）

> 目标：**小改动不触发重型构建**；只有在“功能验收完成”后，才允许手动触发重型 CI。

## 1. 术语

- **轻量工作流（Light）**：不做全量编译/构建产物发布；允许在 PR / push 上运行（例如：YAML 语法/文档检查、策略检查、纯脚本校验）。
- **重型工作流（Heavy）**：会触发较长时间的 Rust/Flutter 编译、E2E、发布打包等；**默认必须仅允许 `workflow_dispatch`**（或发布 tag 推送等“明确意图”的触发）。

## 2. 门禁机制（Feature Gate）

重型工作流统一使用：

1) `workflow_dispatch` inputs：
- `feature_ready`：必须为 `true`
- `feature_gate_file`：必须提供 gate 文件路径（例如 `.ci/feature-gates/app-channel-v1.yaml`）

2) 入口校验脚本：
- `scripts/ci/feature_gate_guard.sh`

该脚本要求 gate 文件至少包含：
- `acceptance_checked: true`
- `ready_for_build: true`

### Gate 文件模板

建议每个“可触发重构建”的功能对应一个 gate 文件：

```yaml
feature_id: app-channel-v1
owner: sikuai
acceptance_checked: true
ready_for_build: true
timestamp: 2026-02-27T00:00:00+08:00
notes: "Flutter app skeleton + backend API v1 ready"
```

## 3. 当前已门禁化的重型工作流（示例）

以下工作流入口已调整为 `workflow_dispatch` 并在首个 job 执行 feature gate 校验：

- `.github/workflows/ci-run.yml`
- `.github/workflows/ci-build-fast.yml`
- `.github/workflows/test-e2e.yml`

> 注：`pub-ubuntu-build.yml` 保留 tag 推送发布入口（`push.tags: v*`），这是“明确意图触发”；其 `workflow_dispatch` 路径仍执行 feature gate 校验。

## 4. 为什么不使用 `ALLOW_ACTIONS.txt` 作为 Actions 门禁

仓库根目录的 `ALLOW_ACTIONS.txt` 是 **Autopilot 本地策略**：用于约束本地是否允许执行 `git push`/触发远端 Actions。

- 它**不适合作为 GitHub Actions 内部门禁**：因为 Actions 的触发发生在远端，依赖“文件存在且已 push”会造成耦合与误触发风险。
- 远端门禁应使用 **workflow 触发条件（`workflow_dispatch`）+ feature gate 文件内容校验**。

## 5. 后续工作（待完成）

### 5.1 仍在 `push`/`pull_request` 自动触发的 Heavy 候选（需要门禁化）

> 这些 workflow 目前会在 PR / push 上跑较重的 build/test/scan。建议按下表逐个改造：

| Workflow | 现状风险（为何重） | 建议门禁策略（推荐） |
|---|---|---|
| `.github/workflows/feature-matrix.yml` | `cargo check/test` 矩阵，最重；PR 上跑会放大成本 | **拆分 Light/Heavy**：保留一个 Light（例如 actionlint/脚本审计/`cargo metadata`）用于 PR；矩阵 Heavy 改为 `workflow_dispatch + feature gate`，可选加 nightly `schedule` |
| `.github/workflows/ci-reproducible-build.yml` | repro build probe（构建链路长） | 改为 `workflow_dispatch + feature gate`；可选保留 `schedule`（每日/每周） |
| `.github/workflows/ci-supply-chain-provenance.yml` | release-fast build + cosign/provenance（链路长、依赖外部） | 保留 **release/tag** 等“明确意图”触发；其余改为 `workflow_dispatch + feature gate` |
| `.github/workflows/sec-codeql.yml` | CodeQL + `cargo build --workspace --all-targets --locked`（重） | 建议：保留 `schedule`（每周）+ `workflow_dispatch + feature gate`；移除 PR/push 自动触发（或仅在安全相关路径变更时触发） |
| `.github/workflows/sec-audit.yml` | `cargo-deny`/`gitleaks`/安全回归（偏重） | 建议拆分：将 **超轻量** 的 secret 扫描/策略检查留在 PR；其余（deny/全量回归）改为 `workflow_dispatch + feature gate` + `schedule` |
| `.github/workflows/pub-docker-img.yml` | PR docker smoke build 镜像（重，且易造成误触发） | 改为 `workflow_dispatch + feature gate`；或增加严格 `paths:` 仅在 Docker/部署相关文件变更时触发 |

### 5.2 改造落地顺序（建议）

1. `feature-matrix.yml`（收益最大）
2. `sec-codeql.yml` / `sec-audit.yml`（成本高且频繁触发）
3. `ci-reproducible-build.yml` / `ci-supply-chain-provenance.yml`
4. `pub-docker-img.yml`

### 5.3 轻量 workflow 范围（允许保留自动触发）

建议允许在 PR/push 自动触发的 Light 范围（需要持续审计，防止被“塞回重任务”）：

- `workflow-sanity.yml`（actionlint/no-tabs 等）
- `ci-change-audit.yml`（CI helper 脚本测试 + 审计）
- `ci-provider-connectivity.yml`（小脚本探测）
- `pr-label-policy-check.yml`（策略一致性检查）
- `docs-deploy.yml`（仅 docs 路径触发；必须配置严格 `paths:`）

## 6. 手动触发说明（运维/发布）

在 GitHub Actions 页面手动触发（Run workflow）时：

- `feature_ready=true`
- `feature_gate_file=.ci/feature-gates/<feature>.yaml`

若 gate 校验失败，工作流会在最前置步骤快速失败，避免浪费 runner 资源。
