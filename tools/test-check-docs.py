#!/usr/bin/env python3
"""Positive and negative controls for the maintained documentation checker."""

from pathlib import Path
import runpy
import tempfile
import unittest

CHECK = runpy.run_path(str(Path(__file__).with_name("check-docs.py")))


class Links(unittest.TestCase):
    def test_existing_links_and_encoded_anchor(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            guide = root / "a guide.md"
            guide.write_text("# A `type` & its owner\n\n# Repeat\n\n# Repeat\n")
            index = root / "README.md"
            index.write_text(
                "[guide](a%20guide.md#a-type--its-owner)\n"
                "[again](<a guide.md#repeat-1>)\n[directory](.)\n"
                "[external](https://example.com/missing#anchor)\n"
            )
            count, failures = CHECK["check"](root, [index])
            self.assertEqual(count, 3)
            self.assertEqual(failures, [])

    def test_missing_path_anchor_and_escape_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            index = root / "README.md"
            index.write_text(
                "# Present\n[bad](missing.md)\n"
                "[bad](#absent)\n[bad](../outside.md)\n"
            )
            count, failures = CHECK["check"](root, [index])
            self.assertEqual(count, 3)
            self.assertEqual(len(failures), 3)
            for message, failure in zip(
                ["missing target", "missing anchor", "escapes repository"], failures
            ):
                self.assertIn(message, failure)

    def test_wrapped_labels_still_check_targets_and_anchors(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "guide.md").write_text("# Present\n")
            index = root / "README.md"
            index.write_text(
                "[a wrapped\nlabel](guide.md#present)\n\n"
                "[missing\nfile](missing.md)\n\n"
                "[missing\nanchor](guide.md#absent)\n"
            )
            count, failures = CHECK["check"](root, [index])
            self.assertEqual(count, 3)
            self.assertEqual(len(failures), 2)
            self.assertIn("README.md:4: missing target", failures[0])
            self.assertIn("README.md:7: missing anchor", failures[1])

    def test_fenced_examples_do_not_create_links_or_headings(self):
        text = "# Real\n```md\n# Fake\n[bad](missing.md)\n```\n[good](#real)\n"
        self.assertEqual(CHECK["anchors"](text), {"real"})
        self.assertEqual(list(CHECK["links"](text)), [(6, "#real")])

    def test_reference_links_are_resolved_or_rejected(self):
        text = "[one][TARGET]\n[two][]\n[missing][absent]\n[target]: guide.md\n[two]: #here\n"
        self.assertEqual(
            [target for _, target in CHECK["links"](text)],
            ["guide.md", "#here", "missing-reference:absent"],
        )

    def test_duplicate_heading_slugs_and_explicit_anchors(self):
        text = '# Same\n# Same\n# Same-1\n<a id="custom"></a>\n'
        self.assertEqual(
            CHECK["anchors"](text), {"same", "same-1", "same-1-1", "custom"}
        )


if __name__ == "__main__":
    unittest.main()
