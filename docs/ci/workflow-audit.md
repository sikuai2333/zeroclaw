# Workflow 触发审计（M1）

时间：2026-02-27

目标：找出仍在 `push` / `pull_request`（或 `pull_request_target`）上触发的 workflow，并标注其中可能属于 **Heavy**（会跑较重的 build/test/docker）的项目，作为下一步“门禁化 / Light-Heavy 拆分”的输入。

> 说明：本审计只基于静态扫描（触发条件 + 关键命令/Action 关键词），最终还需要逐个 workflow 结合实际 job/if 条件确认。

## 1) 存在 push 触发的 workflows

- ci-change-audit.yml
- ci-provider-connectivity.yml
- ci-reproducible-build.yml
- ci-supply-chain-provenance.yml
- docs-deploy.yml
- feature-matrix.yml
- pr-label-policy-check.yml
- pub-docker-img.yml
- pub-prerelease.yml
- pub-release.yml
- pub-ubuntu-build.yml
- sec-audit.yml
- sec-codeql.yml
- workflow-sanity.yml

## 2) 存在 pull_request 触发的 workflows

- ci-change-audit.yml
- ci-provider-connectivity.yml
- ci-reproducible-build.yml
- docs-deploy.yml
- feature-matrix.yml
- main-promotion-gate.yml
- pr-label-policy-check.yml
- pub-docker-img.yml
- sec-audit.yml
- sec-codeql.yml
- workflow-sanity.yml

## 3) 存在 pull_request_target 触发的 workflows

- pr-auto-response.yml
- pr-intake-checks.yml
- pr-labeler.yml

## 4) Heavy 信号（静态关键词）

### 4.1 Rust build/test 相关关键词命中

（命中不代表一定 Heavy，但通常意味着跑编译/测试）

- pub-release.yml：`cargo build --profile release-fast --locked ...`
- pub-ubuntu-build.yml：`cargo build --profile release-fast --locked ...`
- sec-codeql.yml：`cargo build --workspace --all-targets --locked`
- pub-prerelease.yml：`cargo build --profile release-fast --locked ...`
- feature-matrix.yml / nightly-all-features.yml：`cargo test --locked ... agent_e2e ...`

### 4.2 Docker build/push 相关关键词命中

- pub-docker-img.yml：build-push-action / docker/login-action

## 5) 下一步改造建议（待执行）

- 将明显 Heavy 的 workflow（例如 release/build/docker/codeql）从 `push/pull_request` 触发迁移为：
  - `workflow_dispatch + feature gate`（默认）
  - 或保留少数“明确意图”的触发（例如 tag push 的发布），其余路径门禁
- 为 PR 流保留 Light 工作流（如策略检查、文档链接检查、YAML sanity）

