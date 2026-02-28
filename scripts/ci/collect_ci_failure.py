#!/usr/bin/env python3
"""Collect GitHub Actions failure evidence and (optionally) update CI_FAILURES.md.

This script is designed for the Flutter App Channel revamp M4 loop:
- After a *single controlled* heavy build is triggered (workflow_dispatch + feature gate),
  we must fetch the run result, extract failure logs, and write a concise snapshot to
  CI_FAILURES.md.

Modes
1) Online mode (requires `gh` authenticated):
   - Fetch run metadata + download logs.zip via GitHub API.
2) Offline mode (no GitHub access needed):
   - Parse a provided logs.zip file and render the same markdown snapshot.

Notes
- This script does NOT trigger any workflow. It only reads run metadata + downloads logs.

Examples
  # Online dry-run: print extracted summary to stdout
  python3 scripts/ci/collect_ci_failure.py --repo sikuai2333/zeroclaw --run-id 22440621743

  # Online update CI_FAILURES.md in-place
  python3 scripts/ci/collect_ci_failure.py --repo sikuai2333/zeroclaw --run-id 22440621743 --write

  # Offline dry-run: parse a local logs zip (optionally provide run id/url for nicer snapshot)
  python3 scripts/ci/collect_ci_failure.py --logs-zip /path/to/run-22440621743-logs.zip --repo sikuai2333/zeroclaw --run-id 22440621743

  # Offline with explicit metadata
  python3 scripts/ci/collect_ci_failure.py --logs-zip run-22440621743.zip --meta-json run_meta.json --write
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import re
import subprocess
import sys
import tempfile
import textwrap
import zipfile
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable


RUN_URL_RE = re.compile(r"/actions/runs/(?P<id>\d+)")
ANSI_ESCAPE_RE = re.compile(r"\x1b\[[0-9;]*m")
ECHO_ERROR_LITERAL_RE = re.compile(r"\becho\s+[\"']::error::", re.IGNORECASE)


@dataclass
class RunMeta:
    repo: str
    run_id: int
    name: str
    html_url: str
    status: str
    conclusion: str
    created_at: str


def sh(
    cmd: list[str],
    *,
    check: bool = True,
    capture: bool = True,
    text: bool = True,
) -> subprocess.CompletedProcess:
    env = os.environ.copy()
    env.setdefault("GH_PAGER", "cat")
    env.setdefault("PAGER", "cat")
    return subprocess.run(
        cmd,
        check=check,
        capture_output=capture,
        text=text,
        env=env,
    )


def gh_json(args: list[str]) -> dict:
    """Call `gh api ...` and parse JSON."""
    # Avoid `--silent` here: it can suppress body output in some gh versions.
    cp = sh(["gh", "api", "--header", "Accept: application/vnd.github+json", *args])
    body = cp.stdout.strip()
    if not body:
        # Fallback: header-less request in case specific gh builds behave oddly.
        cp = sh(["gh", "api", *args])
        body = cp.stdout.strip()
    if not body:
        raise RuntimeError(
            "gh api returned empty body. Verify `gh auth status` and endpoint arguments."
        )
    try:
        return json.loads(body)
    except json.JSONDecodeError as e:
        raise RuntimeError(
            f"Failed to parse JSON from gh api. stderr={cp.stderr.strip()}"
        ) from e


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(
        description="Collect GitHub Actions failure evidence and optionally update CI_FAILURES.md",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=textwrap.dedent(
            """
            Tips:
            - If you don't know the run id yet, find it via:
                gh run list -R OWNER/REPO --workflow <workflow.yml> --limit 5

            Offline usage:
              1) In an environment that can access GitHub:
                   gh api repos/OWNER/REPO/actions/runs/<run_id>/logs > run-<run_id>-logs.zip
              2) Then parse locally (no network needed):
                   python3 scripts/ci/collect_ci_failure.py --logs-zip run-<run_id>-logs.zip --repo OWNER/REPO --run-id <run_id>
            """
        ),
    )

    # Repo is required in online mode; optional in offline mode.
    p.add_argument("--repo", required=False, help="OWNER/REPO, e.g. sikuai2333/zeroclaw")

    # Online selectors (optional when using --logs-zip)
    p.add_argument("--run-id", type=int, help="GitHub Actions run id")
    p.add_argument("--run-url", help="Run URL like https://github.com/OWNER/REPO/actions/runs/<id>")

    # Offline selector
    p.add_argument("--logs-zip", help="Local path to run logs.zip (offline mode)")

    p.add_argument(
        "--meta-json",
        help=(
            "Optional JSON file to provide run metadata in offline mode. "
            "Fields: repo, run_id, name, html_url, status, conclusion, created_at"
        ),
    )

    p.add_argument(
        "--ci-failures",
        default="CI_FAILURES.md",
        help="Path to CI_FAILURES.md (default: CI_FAILURES.md)",
    )
    p.add_argument(
        "--next-action",
        default="NEXT_ACTION.md",
        help="Path to NEXT_ACTION.md (default: NEXT_ACTION.md)",
    )
    p.add_argument(
        "--write",
        action="store_true",
        help="Write/update CI_FAILURES.md (default: dry-run prints summary)",
    )
    p.add_argument(
        "--write-next-action",
        action="store_true",
        help="Write/update NEXT_ACTION.md with the latest CI failure follow-up action",
    )
    p.add_argument(
        "--max-snippets",
        type=int,
        default=3,
        help="Max number of log snippets to include (default: 3)",
    )
    p.add_argument(
        "--max-lines",
        type=int,
        default=30,
        help="Max lines per snippet (default: 30)",
    )
    return p.parse_args()


def resolve_run_id(args: argparse.Namespace) -> int:
    if args.run_id is not None:
        return int(args.run_id)
    m = RUN_URL_RE.search(args.run_url or "")
    if not m:
        raise SystemExit(f"Invalid --run-url: {args.run_url}")
    return int(m.group("id"))


def fetch_run_meta(repo: str, run_id: int) -> RunMeta:
    data = gh_json([f"repos/{repo}/actions/runs/{run_id}"])
    return RunMeta(
        repo=repo,
        run_id=run_id,
        name=str(data.get("name") or data.get("workflow_id") or "<unknown>"),
        html_url=str(data.get("html_url") or f"https://github.com/{repo}/actions/runs/{run_id}"),
        status=str(data.get("status") or "unknown"),
        conclusion=str(data.get("conclusion") or "unknown"),
        created_at=str(data.get("created_at") or ""),
    )


def load_run_meta_json(path: Path) -> RunMeta:
    data = json.loads(path.read_text(encoding="utf-8"))
    # Be tolerant: allow missing fields and fill defaults.
    repo = str(data.get("repo") or "<unknown>")
    run_id = int(data.get("run_id") or 0)
    return RunMeta(
        repo=repo,
        run_id=run_id,
        name=str(data.get("name") or "<unknown>"),
        html_url=str(data.get("html_url") or ""),
        status=str(data.get("status") or "unknown"),
        conclusion=str(data.get("conclusion") or "unknown"),
        created_at=str(data.get("created_at") or ""),
    )


def download_logs_zip(repo: str, run_id: int, out_path: Path) -> None:
    """Download the run logs zip to out_path."""
    cmd = [
        "gh",
        "api",
        "--header",
        "Accept: application/vnd.github+json",
        f"repos/{repo}/actions/runs/{run_id}/logs",
    ]
    env = os.environ.copy()
    env.setdefault("GH_PAGER", "cat")
    env.setdefault("PAGER", "cat")

    errors: list[str] = []
    for attempt in range(1, 4):
        with out_path.open("wb") as out_fp:
            cp = subprocess.run(
                cmd,
                check=False,
                stdout=out_fp,
                stderr=subprocess.PIPE,
                text=False,
                env=env,
            )
        size = out_path.stat().st_size if out_path.exists() else 0
        if cp.returncode == 0 and size > 0:
            return

        stderr_text = (cp.stderr or b"").decode("utf-8", errors="replace").strip()
        errors.append(f"attempt={attempt} rc={cp.returncode} size={size} stderr={stderr_text}")

    raise RuntimeError("Failed to download logs zip via gh api. " + " | ".join(errors))


ERROR_PATTERNS: list[re.Pattern[str]] = [
    re.compile(r"::error::", re.IGNORECASE),
    re.compile(r"\berror:\b", re.IGNORECASE),
    re.compile(r"Process completed with exit code", re.IGNORECASE),
    re.compile(r"needs to be updated but --locked was passed", re.IGNORECASE),
    re.compile(r"panic", re.IGNORECASE),
]


def score_line(line: str) -> int:
    score = 0
    l = line.strip()
    if not l:
        return 0
    # Avoid false positives from shell scripts that print a literal ::error:: string.
    if ECHO_ERROR_LITERAL_RE.search(l):
        return 0
    if "needs to be updated but --locked was passed" in l:
        score += 50
    if "::error::" in l.lower():
        score += 20
    if re.search(r"\berror:\b", l, re.IGNORECASE):
        score += 10
    if "exit code" in l.lower():
        score += 8
    if "warning" in l.lower():
        score += 1
    return score


def strip_ansi(text: str) -> str:
    return ANSI_ESCAPE_RE.sub("", text)


def iter_log_text_files(z: zipfile.ZipFile) -> Iterable[tuple[str, str]]:
    for name in z.namelist():
        # logs usually have .txt entries (but can be others)
        if name.endswith("/"):
            continue
        if not (name.endswith(".txt") or name.endswith(".log") or name.endswith(".json")):
            continue
        try:
            raw = z.read(name)
        except KeyError:
            continue
        # be robust with encoding
        text = raw.decode("utf-8", errors="replace")
        yield name, text


def extract_snippets(zip_path: Path, *, max_snippets: int, max_lines: int) -> list[dict]:
    """Return best snippets across all files.

    Each snippet: {file, lines:[...], score}
    """
    snippets: list[dict] = []
    with zipfile.ZipFile(zip_path) as z:
        for fname, text in iter_log_text_files(z):
            lines = text.splitlines()
            # collect candidate windows around high-signal lines
            for i, line in enumerate(lines):
                if not any(p.search(line) for p in ERROR_PATTERNS):
                    continue
                s = score_line(line)
                if s <= 0:
                    continue
                start = max(0, i - 6)
                end = min(len(lines), i + 6)
                window = lines[start:end]
                window = [strip_ansi(item) for item in window]
                # trim window length
                if len(window) > max_lines:
                    window = window[:max_lines]
                snippets.append({"file": fname, "lines": window, "score": s})

    # merge duplicates (same file + same first line)
    dedup: dict[tuple[str, str], dict] = {}
    for sn in snippets:
        key = (sn["file"], sn["lines"][0] if sn["lines"] else "")
        if key not in dedup or sn["score"] > dedup[key]["score"]:
            dedup[key] = sn

    best = sorted(dedup.values(), key=lambda x: x["score"], reverse=True)
    return best[:max_snippets]


def render_section(meta: RunMeta, snippets: list[dict]) -> str:
    now = dt.datetime.now(dt.timezone(dt.timedelta(hours=8)))
    ts = now.strftime("%Y-%m-%d %H:%M %z")

    is_success = (meta.conclusion or "").lower() == "success"
    if is_success:
        root_md = "- 本次 run 结论为 success，未检测到失败根因。"
    else:
        # Root-cause heuristic: pick a high-signal line from top snippets.
        root_lines = extract_root_cause_lines(snippets)
        root_md = (
            "\n".join(f"- `{l}`" for l in root_lines)
            if root_lines
            else "- (未从日志片段中自动识别到唯一 root cause；请查看 snippets)"
        )

    snippets_md_parts: list[str] = []
    for idx, sn in enumerate(snippets, start=1):
        body = "\n".join(sn["lines"]).strip()
        snippets_md_parts.append(
            "\n".join(
                [
                    f"### Snippet {idx}",
                    f"- File: `{sn['file']}`",
                    f"- Score: {sn['score']}",
                    "",
                    "```text",
                    body,
                    "```",
                ]
            ).strip()
        )

    snippets_md = "\n\n".join(snippets_md_parts) if snippets_md_parts else "(No snippets extracted.)"

    section = (
        "\n".join(
            [
                "## Snapshot",
                f"- Time: {ts}",
                f"- Source repo: {meta.repo}",
                f"- Run ID: {meta.run_id}",
                f"- Workflow: {meta.name}",
                f"- Status: {meta.status}",
                f"- Conclusion: {meta.conclusion}",
                f"- URL: {meta.html_url}",
                "",
                "## Root Cause (from run logs)",
                root_md,
                "",
                "## Log Snippets (auto-extracted)",
                snippets_md,
            ]
        ).strip()
        + "\n"
    )

    return section


def extract_root_cause_lines(snippets: list[dict]) -> list[str]:
    root_lines: list[str] = []
    for sn in snippets[:1]:
        for line in sn.get("lines", []):
            if "needs to be updated but --locked was passed" in line:
                root_lines.append(line.strip())
                break
        if root_lines:
            break

    if not root_lines and snippets:
        # fallback: first non-empty line that includes error/exit-code
        for line in snippets[0].get("lines", []):
            if re.search(r"::error::|\berror:\b|exit code", line, re.IGNORECASE):
                root_lines.append(line.strip())
                break
    return root_lines


def update_ci_failures(path: Path, new_section: str) -> None:
    header = "# CI_FAILURES\n\n"
    if not path.exists():
        path.write_text(header, encoding="utf-8")

    old = path.read_text(encoding="utf-8")
    if not old.lstrip().startswith("# CI_FAILURES"):
        old = header + old

    out_lines: list[str] = []
    out_lines.append("# CI_FAILURES")
    out_lines.append("")
    out_lines.append(new_section.rstrip())
    out_lines.append("")
    path.write_text("\n".join(out_lines), encoding="utf-8")


def render_next_action(meta: RunMeta, root_lines: list[str]) -> str:
    if (meta.conclusion or "").lower() == "success":
        return textwrap.dedent(
            f"""\
            # NEXT_ACTION

            - 最近一次受控构建已成功（Run ID: {meta.run_id}, Workflow: {meta.name}）
              1) 记录本次成功基线并归档 run 链接（用于后续回归对比）
              2) 推进下一里程碑任务（M5：WS 事件类型与前端实时订阅）
              3) 继续遵守策略：仅在功能完成后单次触发 workflow_dispatch
            """
        )

    root = (
        root_lines[0]
        if root_lines
        else "未自动识别出明确 root cause，请先查看 CI_FAILURES.md 的 snippets。"
    )
    return textwrap.dedent(
        f"""\
        # NEXT_ACTION

        - CI 失败回收已完成，优先修复最新失败构建（Run ID: {meta.run_id}, Workflow: {meta.name}）
          1) 根因：`{root}`
          2) 在本地最小复现并修复（优先 `cargo check --locked` / 对应失败步骤）
          3) 修复后更新 feature gate 文件（仅功能完成后）
          4) 仅触发一次受控 workflow_dispatch 复验，并再次回收结果
        """
    )


def update_next_action(path: Path, content: str) -> None:
    path.write_text(content.rstrip() + "\n", encoding="utf-8")


def build_offline_meta(args: argparse.Namespace, run_id: int) -> RunMeta:
    # Prefer meta-json if provided.
    if args.meta_json:
        m = load_run_meta_json(Path(args.meta_json))
        # allow args overrides when meta is missing
        if m.run_id == 0:
            m.run_id = run_id
        if (not m.repo or m.repo == "<unknown>") and args.repo:
            m.repo = args.repo
        if not m.html_url and args.run_url:
            m.html_url = args.run_url
        return m

    repo = args.repo or "<unknown>"
    html_url = args.run_url or (f"https://github.com/{repo}/actions/runs/{run_id}" if repo != "<unknown>" and run_id else "")
    return RunMeta(
        repo=repo,
        run_id=run_id,
        name="<offline logs.zip>",
        html_url=html_url,
        status="unknown",
        conclusion="unknown",
        created_at="",
    )


def main() -> int:
    args = parse_args()

    # Validate selector combinations.
    has_online_selector = args.run_id is not None or bool(args.run_url)
    has_offline_selector = bool(args.logs_zip)

    if not has_online_selector and not has_offline_selector:
        print("ERROR: Must provide either --logs-zip (offline) or --run-id/--run-url (online).", file=sys.stderr)
        return 2

    if has_offline_selector:
        # Offline mode: no gh required.
        run_id = 0
        if has_online_selector:
            # allow providing id/url to enrich snapshot
            try:
                run_id = resolve_run_id(args)
            except SystemExit:
                run_id = int(args.run_id or 0)
        meta = build_offline_meta(args, run_id)
        zip_path = Path(args.logs_zip)
        if not zip_path.exists():
            print(f"ERROR: logs zip not found: {zip_path}", file=sys.stderr)
            return 2
        snippets = extract_snippets(zip_path, max_snippets=args.max_snippets, max_lines=args.max_lines)
        section = render_section(meta, snippets)

        if not args.write and not args.write_next_action:
            print(section)
            return 0

        root_lines = extract_root_cause_lines(snippets)
        if args.write:
            ci_failures_path = Path(args.ci_failures)
            update_ci_failures(ci_failures_path, section)
            print(f"Updated {ci_failures_path}")
        if args.write_next_action:
            next_action_path = Path(args.next_action)
            update_next_action(next_action_path, render_next_action(meta, root_lines))
            print(f"Updated {next_action_path}")
        if not args.write:
            print(section)
        return 0

    # Online mode
    if not args.repo:
        print("ERROR: --repo is required unless --logs-zip is provided.", file=sys.stderr)
        return 2

    run_id = resolve_run_id(args)

    # fast preflight
    try:
        sh(["gh", "auth", "status"], check=False)
    except FileNotFoundError:
        print("ERROR: gh CLI not found in PATH.", file=sys.stderr)
        return 2

    meta = fetch_run_meta(args.repo, run_id)

    with tempfile.TemporaryDirectory(prefix="ci_failure_") as td:
        zip_path = Path(td) / f"run-{run_id}-logs.zip"
        download_logs_zip(args.repo, run_id, zip_path)
        snippets = extract_snippets(zip_path, max_snippets=args.max_snippets, max_lines=args.max_lines)

    section = render_section(meta, snippets)

    if not args.write and not args.write_next_action:
        print(section)
        return 0

    root_lines = extract_root_cause_lines(snippets)
    if args.write:
        ci_failures_path = Path(args.ci_failures)
        update_ci_failures(ci_failures_path, section)
        print(f"Updated {ci_failures_path}")
    if args.write_next_action:
        next_action_path = Path(args.next_action)
        update_next_action(next_action_path, render_next_action(meta, root_lines))
        print(f"Updated {next_action_path}")
    if not args.write:
        print(section)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
