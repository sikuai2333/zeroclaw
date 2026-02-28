# Feature Gate Files

Use this directory to control heavy CI builds.

Rules:
1. Small pushes must not trigger heavy compile workflows.
2. Heavy compile workflows run only when a feature gate file is explicitly provided and validated.
3. `acceptance_checked` and `ready_for_build` must both be `true`.

Template:
- Copy `example-feature.yaml`
- Rename it per feature, e.g. `app-channel-v1.yaml`
- Fill owner, scope, and acceptance evidence

Example manual trigger inputs:
- `feature_ready=true`
- `feature_gate_file=.ci/feature-gates/example-feature.yaml` (safe default)
- custom gate example: `feature_gate_file=.ci/feature-gates/app-channel-v1.yaml`
