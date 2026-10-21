#!/usr/bin/env python3
"""Regression tests for the CI cfg(test) unwrap/expect filter."""

import importlib.util
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "ci_filter_testcode.py"
SPEC = importlib.util.spec_from_file_location("ci_filter_testcode", SCRIPT)
FILTER = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(FILTER)


class FindTestLinesTests(unittest.TestCase):
    def test_excludes_hits_inside_nested_cfg_test_module_only(self):
        source = """fn production() {
    value.unwrap();
}
#[cfg(test)]
mod tests {
    fn nested() {
        if true {
            fixture.expect("test");
        }
    }
}
"""
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "sample.rs"
            path.write_text(source)
            test_lines = FILTER.find_test_lines(str(path))
        lines = source.splitlines()
        production_line = next(i for i, line in enumerate(lines, 1) if "value.unwrap" in line)
        fixture_line = next(i for i, line in enumerate(lines, 1) if "fixture.expect" in line)
        self.assertNotIn(production_line, test_lines)
        self.assertIn(fixture_line, test_lines)

    def test_compound_cfg_test_module_is_excluded(self):
        source = """fn production() {
    value.unwrap();
}
#[cfg(all(test, target_os = "linux"))]
mod tests {
    fn test_only() {
        fixture.expect("test");
    }
}
"""
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "sample.rs"
            path.write_text(source)
            test_lines = FILTER.find_test_lines(str(path))
        lines = source.splitlines()
        production_line = next(i for i, line in enumerate(lines, 1) if "value.unwrap" in line)
        fixture_line = next(i for i, line in enumerate(lines, 1) if "fixture.expect" in line)
        self.assertNotIn(production_line, test_lines)
        self.assertIn(fixture_line, test_lines)

    def test_cfg_test_module_declaration_does_not_swallow_following_code(self):
        source = """#[cfg(test)] mod tests;
fn production() {
    value.unwrap();
}
"""
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "sample.rs"
            path.write_text(source)
            test_lines = FILTER.find_test_lines(str(path))
        production_line = next(
            i for i, line in enumerate(source.splitlines(), 1) if "value.unwrap" in line
        )
        self.assertNotIn(production_line, test_lines)

    def test_missing_source_file_is_safe(self):
        with tempfile.TemporaryDirectory() as temp:
            missing = Path(temp) / "missing.rs"
            self.assertEqual(FILTER.find_test_lines(str(missing)), set())

    def test_closing_brace_in_a_string_literal_does_not_end_the_test_block(self):
        # A '}' inside a string is not a block delimiter. Reading it as one
        # ends the cfg(test) block early, and every later test line is then
        # reported as a production violation.
        source = '''#[cfg(test)]
mod tests {
    fn t() {
        let marker = "\\n  }";
        marker.unwrap();
    }
    fn later() {
        later_value.expect("later");
    }
}
'''
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "sample.rs"
            path.write_text(source)
            test_lines = FILTER.find_test_lines(str(path))
        lines = source.splitlines()
        first = next(i for i, line in enumerate(lines, 1) if "marker.unwrap" in line)
        later = next(i for i, line in enumerate(lines, 1) if "later_value.expect" in line)
        self.assertIn(first, test_lines)
        self.assertIn(later, test_lines)

    def test_opening_brace_in_a_string_literal_does_not_hide_production_code(self):
        # A '{' inside a string is not a block delimiter. Reading it as one
        # extends the test block past its end, and a real production
        # violation after the module is then hidden from the gate.
        source = '''#[cfg(test)]
mod tests {
    fn t() {
        let marker = "{";
        marker.unwrap();
    }
}

fn production() {
    value.unwrap();
}
'''
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "sample.rs"
            path.write_text(source)
            test_lines = FILTER.find_test_lines(str(path))
        lines = source.splitlines()
        production_line = next(i for i, line in enumerate(lines, 1) if "value.unwrap" in line)
        self.assertNotIn(production_line, test_lines)

    def test_raw_string_and_comment_braces_are_ignored(self):
        source = '''#[cfg(test)]
mod tests {
    fn t() {
        let raw = r#"a " brace"#;
        let hash = r#"}"#;
        // a } in a line comment
        /* a { in a block comment */
        raw.unwrap();
    }
}

fn production() {
    value.unwrap();
}
'''
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "sample.rs"
            path.write_text(source)
            test_lines = FILTER.find_test_lines(str(path))
        lines = source.splitlines()
        test_line = next(i for i, line in enumerate(lines, 1) if "raw.unwrap" in line)
        production_line = next(i for i, line in enumerate(lines, 1) if "value.unwrap" in line)
        self.assertIn(test_line, test_lines)
        self.assertNotIn(production_line, test_lines)

    def test_char_literal_brace_does_not_shift_the_block(self):
        # '{' and not '}': with '}' the old filter merely ends the block one
        # line later, so both assertions still hold and the test proves
        # nothing. With '{' the old filter extends the block past its end and
        # the production line is hidden - this is the case that fails it.
        source = """#[cfg(test)]
mod tests {
    fn t() {
        let c = '{';
        c.unwrap();
    }
}

fn production() {
    value.unwrap();
}
"""
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "sample.rs"
            path.write_text(source)
            test_lines = FILTER.find_test_lines(str(path))
        lines = source.splitlines()
        test_line = next(i for i, line in enumerate(lines, 1) if "c.unwrap" in line)
        production_line = next(i for i, line in enumerate(lines, 1) if "value.unwrap" in line)
        self.assertIn(test_line, test_lines)
        self.assertNotIn(production_line, test_lines)

    def test_nested_block_comment_does_not_leak_a_brace(self):
        """Rust nests block comments. Stopping at the first */ leaves a
        spurious { in the code, which extends the test block past its end
        and hides real production code.

        The comment has to be INSIDE the test module for that: the filter
        counts depth from the brace that opens the module, so a stray brace
        before the module does not extend it.
        """
        source = """#[cfg(test)]
mod tests {
    /* /* } */ { */
    fn t() {
        raw.unwrap();
    }
}

fn production() {
    value.unwrap();
}
"""
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "sample.rs"
            path.write_text(source)
            test_lines = FILTER.find_test_lines(str(path))
        lines = source.splitlines()
        test_line = next(i for i, line in enumerate(lines, 1) if "raw.unwrap" in line)
        production_line = next(i for i, line in enumerate(lines, 1) if "value.unwrap" in line)
        self.assertIn(test_line, test_lines)
        self.assertNotIn(production_line, test_lines)

    def test_cfg_attribute_inside_a_string_is_not_an_attribute(self):
        """`#[cfg(test)]` in a string literal is text, not an attribute.
        Treating it as one makes the block it appears to open swallow the
        production code that follows it.

        The string has to end without a `;` for this to bite: the filter
        skips a cfg attribute whose block is a declaration rather than a
        block, and it finds that semicolon in the raw text.
        """
        source = """fn helper() -> &'static str {
    "#[cfg(test)] {"
}
fn production() {
    value.unwrap();
}
"""
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "sample.rs"
            path.write_text(source)
            test_lines = FILTER.find_test_lines(str(path))
        lines = source.splitlines()
        production_line = next(i for i, line in enumerate(lines, 1) if "value.unwrap" in line)
        self.assertNotIn(production_line, test_lines)


if __name__ == "__main__":
    unittest.main(verbosity=2)
