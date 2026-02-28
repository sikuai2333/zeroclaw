#!/usr/bin/env bash
set -euo pipefail

repo=""
run_id=""
workflow_name=""
ci_failures_path="CI_FAILURES.md"
next_action_path="NEXT_ACTION.md"
task_board_path="TASK_BOARD.md"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo)
      repo="${2:-}"
      shift 2
      ;;
    --run-id)
      run_id="${2:-}"
      shift 2
      ;;
    --workflow)
      workflow_name="${2:-}"
      shift 2
      ;;
    --ci-failures)
      ci_failures_path="${2:-CI_FAILURES.md}"
      shift 2
      ;;
    --next-action)
      next_action_path="${2:-NEXT_ACTION.md}"
      shift 2
      ;;
    --task-board)
      task_board_path="${2:-TASK_BOARD.md}"
      shift 2
      ;;
    *)
      echo "Unknown arg: $1" >&2
      exit 2
      ;;
  esac
done

if [[ -z "$repo" ]]; then
  echo "Usage: $0 --repo OWNER/REPO [--run-id ID] [--workflow NAME] [--ci-failures PATH] [--next-action PATH] [--task-board PATH]" >&2
  exit 2
fi

if [[ -z "$run_id" ]]; then
  runs_json="$(gh run list -R "$repo" --status failure --limit 30 --json databaseId,name,workflowName,createdAt,url)"
  run_id="$(
    RUNS_JSON="$runs_json" python3 - "$workflow_name" <<'PY'
import json
import os
import sys

workflow = (sys.argv[1] or '').strip()
raw = os.environ.get("RUNS_JSON", "").strip()
if not raw:
    raise SystemExit(0)

items = json.loads(raw)
if workflow:
    for item in items:
        if item.get('workflowName') == workflow or item.get('name') == workflow:
            print(item.get('databaseId') or '')
            raise SystemExit(0)
if items:
    print(items[0].get('databaseId') or '')
PY
  )"
fi

if [[ -z "$run_id" ]]; then
  echo "No failed run found for repo=$repo workflow=${workflow_name:-<any>}" >&2
  exit 1
fi

echo "Collecting failure feedback for repo=$repo run_id=$run_id"
python3 scripts/ci/collect_ci_failure.py \
  --repo "$repo" \
  --run-id "$run_id" \
  --write \
  --write-next-action \
  --write-task-board \
  --ci-failures "$ci_failures_path" \
  --next-action "$next_action_path" \
  --task-board "$task_board_path"

echo "Done. Updated: $ci_failures_path, $next_action_path, $task_board_path"
