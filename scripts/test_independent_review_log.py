#!/usr/bin/env python3
"""Contract test for scripts/independent_review_log.py.

The log is only worth anything if the collector finds the remarks, puts
them where the log's README says, and never destroys work that is
already there. These cases pin that:

  - a review remark is recognised and its commit, title, finding,
    severity and status are extracted;
  - a comment that is not a review is ignored;
  - a per-commit entry is written where the README says it goes;
  - an entry that already exists is NEVER overwritten;
  - the index is regenerated and is the same every time;
  - the branch commit log is appended to, never rewritten;
  - malformed input fails instead of writing an empty log.

    python3 scripts/test_independent_review_log.py
"""

import importlib.util
import json
import os
import shutil
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "independent_review_log.py")
SPEC = importlib.util.spec_from_file_location("independent_review_log", SCRIPT)
LOG = importlib.util.module_from_spec(SPEC)
if SPEC.loader is None:
    raise SystemExit("cannot load %s" % SCRIPT)
SPEC.loader.exec_module(LOG)

REVIEW_BODY = """## Independent master review — bd417cd9c79492dbe4a75de08186c9bf79f4e759

**Commit:** [Rebuild BACKLOG.md: one pending table](https://github.com/grep999/ramshield/commit/bd417cd9c79492dbe4a75de08186c9bf79f4e759)

**Finding — P2 documentation/evidence accuracy:** the BACKLOG.md header
says the assessed SHA is `2adc966` and labels it `== origin/master`.

**Status:** CONFIRMED

**Evidence:** inspected the exact commit diff returned by GitHub.
"""

OTHER_BODY = """## Some other workstream

Not a review. It mentions bd417cd in passing and must be ignored.
"""

BRANCH_REVIEW_BODY = """## Independent master review — 1111111111111111111111111111111111111111

**Commit:** [A branch commit](https://github.com/grep999/ramshield/commit/1111111111111111111111111111111111111111)

**Finding — P1 something real:** a finding on a commit that is not on master.
"""


def comments():
    return [
        {"id": 111, "created_at": "2026-10-21T14:29:19Z", "body": REVIEW_BODY},
        {"id": 112, "created_at": "2026-10-21T14:30:00Z", "body": OTHER_BODY},
        {"id": 113, "created_at": "2026-10-21T15:00:00Z", "body": BRANCH_REVIEW_BODY},
    ]


def test_a_review_is_recognised():
    r = LOG.parse(REVIEW_BODY)
    assert r is not None, "a review remark must be recognised"
    assert r["sha"] == "bd417cd9c79492dbe4a75de08186c9bf79f4e759", r
    assert "Rebuild BACKLOG.md" in r["title"], r
    assert "BACKLOG.md header" in r["finding"], r
    assert r["severity"] == "P2", r
    assert r["status"] == "CONFIRMED", r


def test_a_comment_that_is_not_a_review_is_ignored():
    assert LOG.parse(OTHER_BODY) is None, "a non-review comment must be ignored"
    assert LOG.parse("") is None
    assert LOG.parse(None) is None


def test_the_entry_goes_where_the_readme_says():
    with tempfile.TemporaryDirectory() as d:
        log_dir = os.path.join(d, "REVIEW_LOG")
        reviews = LOG.collect(comments())
        assert len(reviews) == 2, [r["sha"] for r in reviews]
        for r in reviews:
            path = os.path.join(log_dir, "commits", r["sha"] + ".md")
            assert LOG.write_if_absent(path, LOG.entry_text(r)), path
            assert os.path.isfile(path), path
        with open(os.path.join(log_dir, "commits",
                               "bd417cd9c79492dbe4a75de08186c9bf79f4e759.md")) as f:
            text = f.read()
        assert "**Status:** CONFIRMED" in text, text
        assert "**Severity:** P2" in text, text
        assert "BACKLOG.md header" in text, text
        assert "issuecomment-111" in text, text


def test_an_existing_entry_is_never_overwritten():
    with tempfile.TemporaryDirectory() as d:
        log_dir = os.path.join(d, "REVIEW_LOG")
        sha = "bd417cd9c79492dbe4a75de08186c9bf79f4e759"
        path = os.path.join(log_dir, "commits", sha + ".md")
        os.makedirs(os.path.dirname(path))
        with open(path, "w") as f:
            f.write("HAND WRITTEN REVIEW\n")
        reviews = LOG.collect(comments())
        for r in reviews:
            LOG.write_if_absent(path, LOG.entry_text(r))
        with open(path) as f:
            assert f.read() == "HAND WRITTEN REVIEW\n", "an existing entry was clobbered"


def test_the_index_is_regenerated_and_stable():
    with tempfile.TemporaryDirectory() as d:
        log_dir = os.path.join(d, "REVIEW_LOG")
        reviews = LOG.collect(comments())
        first = LOG.index_text(reviews, log_dir)
        second = LOG.index_text(reviews, log_dir)
        assert first == second, "the index must be regenerated, not appended to"
        assert "bd417cd" in first, first
        assert "issuecomment-111" in first, first
        assert "issuecomment-112" not in first, "a non-review must not be indexed"
        # an entry that exists is linked, one that does not is marked
        os.makedirs(os.path.join(log_dir, "commits"))
        with open(os.path.join(log_dir, "commits",
                               "bd417cd9c79492dbe4a75de08186c9bf79f4e759.md"), "w") as f:
            f.write("x\n")
        third = LOG.index_text(reviews, log_dir)
        assert "commits/bd417cd9c79492dbe4a75de08186c9bf79f4e759.md" in third, third
        assert "not yet written" in third, third


def test_the_branch_log_is_appended_to():
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "BRANCH_COMMITS.md")
        with open(path, "w") as f:
            f.write("# Review Branch Commit Log\n\n## Entries\n")
        commits = [{"sha": "a" * 40, "parent": "b" * 40, "message": "one"}]
        assert LOG.append_branch_commits(path, commits) == 1
        assert LOG.append_branch_commits(path, commits) == 0, "logged twice"
        with open(path) as f:
            text = f.read()
        assert text.count("a" * 40) == 1, text
        assert "### 1." in text, text


def test_malformed_input_fails():
    with tempfile.TemporaryDirectory() as d:
        bad = os.path.join(d, "bad.json")
        with open(bad, "w") as f:
            f.write('{"not": "a list"}')
        try:
            LOG.load_comments(bad)
        except ValueError:
            return
        raise AssertionError("a malformed comments file must fail")


def main():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for t in tests:
        t()
        print("PASS: %s" % t.__name__)
    print("PASS: the independent review log contract holds (%d cases)" % len(tests))
    return 0


if __name__ == "__main__":
    sys.exit(main())
