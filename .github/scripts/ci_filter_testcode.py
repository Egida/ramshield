#!/usr/bin/env python3
"""
ci_filter_testcode.py — Filter rg output to drop lines inside test-only cfg blocks.

Usage:
    rg ... src/ | python3 ci_filter_testcode.py <src_dir>

Reads the rg output (path:line:content) from stdin.
For each unique source file in the hits, reads the file, finds all cfg attributes
whose condition includes `test`, drops hits inside those test-only blocks.
"""

import os
import re
import sys
from collections import defaultdict


def strip_literals(content: str) -> str:
    """Blank out string and char literal contents, preserving every character.

    Brace depth must be counted over code, not over text. A `{` or `}` inside
    a string literal is not a block delimiter: reading one as one ends a
    cfg(test) block early (later test code is then reported as production
    code) or extends it past its end (real production code is then hidden).
    Both directions are wrong, so the literal contents are replaced with
    spaces. Character count and line breaks are preserved, so offsets and
    line numbers still refer to the original file. (Character count, not
    byte count: a multi-byte character becomes one space, so the byte
    offsets shift. Only line numbers are consumed downstream.)
    """
    out = list(content)
    i = 0
    n = len(content)
    while i < n:
        ch = content[i]
        if ch == "/" and i + 1 < n and content[i + 1] == "/":
            # Line comment: ends at the newline.
            j = content.find("\n", i)
            j = n if j == -1 else j
            for k in range(i, j):
                out[k] = " "
            i = j
        elif ch == "/" and i + 1 < n and content[i + 1] == "*":
            # Block comment: ends at the matching */. Rust NESTS block
            # comments, so `/* /* } */ { */` is one comment, not two. Stopping
            # at the first */ leaves ` { */` as code, and that spurious {
            # extends a cfg(test) block past its end - hiding real production
            # code from the gate. So count the nesting depth.
            depth = 1
            j = i + 2
            while j < n and depth > 0:
                if content.startswith("/*", j):
                    depth += 1
                    j += 2
                elif content.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            for k in range(i, j):
                if out[k] != "\n":
                    out[k] = " "
            i = j
        elif ch == '"' or (
            ch == "r"
            and i + 1 < n
            and (content[i + 1] == '"' or content[i + 1] == "#")
        ):
            # String literal. A raw string (r"..", r#".."#) has no escapes and
            # ends at the closing quote followed by its hashes.
            raw = ch == "r"
            j = i + (1 if raw else 0)
            hashes = 0
            if raw:
                while j < n and content[j] == "#":
                    hashes += 1
                    j += 1
                if j >= n or content[j] != '"':
                    # Not a raw string after all (e.g. an identifier).
                    i += 1
                    continue
            j += 1
            while j < n:
                if not raw and content[j] == "\\":
                    j += 2
                    continue
                if content[j] == '"':
                    k = j + 1
                    while k < n and content[k] == "#" and hashes > 0:
                        k += 1
                        hashes -= 1
                    if hashes == 0:
                        j = k
                        break
                j += 1
            for k in range(i, min(j, n)):
                if out[k] != "\n":
                    out[k] = " "
            i = j
        elif ch == "'":
            # Char literal ('a', '\n') or a lifetime ('a). A lifetime has no
            # closing quote, so only blank a literal that closes.
            j = i + 1
            closed = False
            if j < n and content[j] == "\\":
                j += 2
                while j < n and content[j] != "'":
                    j += 1
                closed = j < n
            elif j + 1 < n and content[j + 1] == "'":
                j += 2
                closed = True
            if closed:
                for k in range(i, j):
                    if out[k] != "\n":
                        out[k] = " "
                i = j
            else:
                i += 1
        else:
            i += 1
    return "".join(out)


def find_test_lines(src_path: str) -> set[int]:
    """Return the set of line numbers inside cfg attributes containing test."""
    if not os.path.isfile(src_path):
        return set()
    with open(src_path) as f:
        content = f.read()

    # Count braces over code only: a brace inside a string literal or a
    # comment is not a block delimiter.
    code = strip_literals(content)

    test_lines: set[int] = set()
    # Match both #[cfg(test)] and forms such as #[cfg(all(test, target_os = "linux"))].
    # Match over `code`, not `content`: a `#[cfg(test)]` inside a string
    # literal or a comment is not an attribute, and treating it as one
    # makes the block it appears to open swallow the production code that
    # follows.
    cfg_positions = [
        m.start()
        for m in re.finditer(r"#\[cfg\([^]]*\btest\b[^]]*\)\]", code)
    ]

    for pos in cfg_positions:
        brace_start = code.find("{", pos)
        if brace_start == -1:
            continue
        # #[cfg(test)] mod tests; / #[cfg(test)] use x; declare no block here.
        semi = code.find(";", pos)
        if semi != -1 and semi < brace_start:
            continue
        depth = 0
        line_no = code[:brace_start].count("\n") + 1
        for ch in code[brace_start:]:
            if ch == "\n":
                line_no += 1
            if ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
                if depth == 0:
                    test_lines.add(line_no)
                    break
                if depth < 0:
                    depth = 0
            test_lines.add(line_no)
    return test_lines


def main():
    if len(sys.argv) < 2:
        print("Usage: ci_filter_testcode.py <src_dir>", file=sys.stderr)
        sys.exit(2)
    src_dir = sys.argv[1]

    hits_by_file: dict[str, list[tuple[int, str]]] = defaultdict(list)
    all_lines = sys.stdin.read().splitlines()

    for line in all_lines:
        line = line.rstrip("\n")
        if not line:
            continue
        parts = line.split(":", 2)
        if len(parts) < 3:
            print(line)
            continue
        fpath, lno_str, _content = parts
        try:
            lno = int(lno_str)
        except ValueError:
            print(line)
            continue
        hits_by_file[fpath].append((lno, line))

    violations = []
    for fpath, hits in hits_by_file.items():
        abs_path = os.path.join(src_dir, fpath)
        if not os.path.isfile(abs_path):
            abs_path = os.path.join(src_dir, os.path.basename(fpath))
        test_lines = find_test_lines(abs_path)
        for lno, line in hits:
            if lno not in test_lines:
                violations.append(line)

    for violation in violations:
        print(violation)


if __name__ == "__main__":
    main()
