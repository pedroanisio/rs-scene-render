"""Reuse the latest successful main CI for exactly the release commit."""

import json
import os
import subprocess
import time


def select_run(runs, sha):
    eligible = [r for r in runs if r.get("head_sha") == sha
                and r.get("head_branch") == "main" and r.get("event") == "push"
                and r.get("path") == ".github/workflows/ci.yml"]
    return max(eligible, key=lambda r: r["id"], default=None)


def has_validated_tools(artifacts):
    return any(a.get("name") == "validated-linux-tools" and not a.get("expired", True)
               for a in artifacts)


def api(path):
    return json.loads(subprocess.check_output(["gh", "api", path], text=True))


def main():
    repository = os.environ["GITHUB_REPOSITORY"]
    sha = os.environ["GITHUB_SHA"]
    endpoint = (f"repos/{repository}/actions/workflows/ci.yml/runs"
                f"?head_sha={sha}&branch=main&event=push&per_page=10")
    deadline = time.monotonic() + 5 * 60 * 60
    run_id = ""
    while True:
        run = select_run(api(endpoint)["workflow_runs"], sha)
        if run is None:
            print("No matching main CI; release will perform full validation.", flush=True)
            break
        if run["status"] == "completed":
            if run["conclusion"] == "cancelled":
                print("Main CI was superseded; performing full release validation.", flush=True)
                break
            if run["conclusion"] != "success":
                raise RuntimeError(f"Main CI {run['id']} did not pass: {run['conclusion']}")
            artifacts = api(f"repos/{repository}/actions/runs/{run['id']}/artifacts")
            if has_validated_tools(artifacts["artifacts"]):
                run_id = str(run["id"])
                print(f"Reusing successful main CI {run_id} for {sha}.", flush=True)
            else:
                print("Validated tools expired or absent; performing full validation.", flush=True)
            break
        if time.monotonic() >= deadline:
            raise TimeoutError(f"Main CI {run['id']} has not completed")
        print(f"Waiting for main CI {run['id']}: {run['status']}", flush=True)
        time.sleep(20)
    with open(os.environ["GITHUB_OUTPUT"], "a") as output:
        print(f"run-id={run_id}", file=output)


if __name__ == "__main__":
    main()
