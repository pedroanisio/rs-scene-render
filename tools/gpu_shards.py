"""Run disjoint GPU integration targets, serially within each CI runner."""

import argparse
import json
import subprocess


def select_targets(names, index, count):
    if count <= 0 or not 0 <= index < count:
        raise ValueError("shard index must be between zero and count - 1")
    return sorted(names)[index::count]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("index", type=int)
    parser.add_argument("count", type=int)
    args = parser.parse_args()
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--locked", "--format-version", "1"],
        text=True,
    ))
    package = next(p for p in metadata["packages"] if p["name"] == "sr-gpu")
    targets = select_targets(
        [t["name"] for t in package["targets"] if "test" in t["kind"]],
        args.index, args.count,
    )
    command = ["cargo", "test", "--locked", "-p", "sr-gpu", "--no-fail-fast"]
    if args.index == 0:
        command.append("--lib")
    for target in targets:
        command.extend(["--test", target])
    if targets or args.index == 0:
        print("GPU targets:", ", ".join(targets), flush=True)
        subprocess.run(command + ["--", "--test-threads=1"], check=True)
    if args.index == 0:
        subprocess.run(["cargo", "test", "--locked", "-p", "sr-gpu", "--doc"],
                       check=True)


if __name__ == "__main__":
    main()
