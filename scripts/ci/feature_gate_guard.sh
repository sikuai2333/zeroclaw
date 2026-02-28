#!/usr/bin/env bash
set -euo pipefail

to_bool() {
  case "${1:-}" in
    true|TRUE|True|1|yes|on) echo "true" ;;
    *) echo "false" ;;
  esac
}

event_name="${GITHUB_EVENT_NAME:-workflow_dispatch}"
ref_name="${GITHUB_REF_NAME:-}"
feature_ready="$(to_bool "${FEATURE_READY:-false}")"
feature_gate_file="${FEATURE_GATE_FILE:-}"

if [[ "$event_name" == "workflow_dispatch" ]]; then
  if [[ "$feature_ready" != "true" ]]; then
    echo "::error::feature_ready is false. Set feature_ready=true only after feature acceptance is complete."
    exit 1
  fi
  if [[ -z "$feature_gate_file" ]]; then
    echo "::error::FEATURE_GATE_FILE is required for workflow_dispatch."
    exit 1
  fi
fi

if [[ -n "$feature_gate_file" ]]; then
  if [[ ! -f "$feature_gate_file" ]]; then
    echo "::error::Feature gate file not found: $feature_gate_file"
    exit 1
  fi

  if ! grep -Eq '^[[:space:]]*acceptance_checked:[[:space:]]*true([[:space:]]|$)' "$feature_gate_file"; then
    echo "::error::Feature gate file must declare acceptance_checked: true"
    exit 1
  fi

  if ! grep -Eq '^[[:space:]]*ready_for_build:[[:space:]]*true([[:space:]]|$)' "$feature_gate_file"; then
    echo "::error::Feature gate file must declare ready_for_build: true"
    exit 1
  fi
fi

if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  {
    echo "feature_ready=$feature_ready"
    echo "feature_gate_file=$feature_gate_file"
    echo "event_name=$event_name"
    echo "ref_name=$ref_name"
  } >> "$GITHUB_OUTPUT"
fi

echo "Feature gate check passed. event=${event_name} ref=${ref_name} gate=${feature_gate_file:-<none>}"
