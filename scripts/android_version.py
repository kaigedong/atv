"""Commit-derived Android release identity; main history must not be rewritten."""
import subprocess


def build_version(root):
    def git(*arguments):
        return subprocess.check_output(["git", *arguments], cwd=root, text=True).strip()
    if git("rev-parse", "--is-shallow-repository") != "false":
        raise ValueError("Release versioning requires complete Git history")
    commit = git("rev-parse", "HEAD")
    code = 1000 + int(git("rev-list", "--first-parent", "--count", commit))
    if not 1 < code <= 2100000000:
        raise ValueError("Android release versionCode is out of range")
    name = f"0.1.0-r{code}"
    return {"sourceCommit": commit, "versionCode": code, "versionName": name, "tag": "v" + name}
