#!/usr/bin/env python3
import runpy
from pathlib import Path
import tempfile
import unittest

manifest = runpy.run_path(str(Path(__file__).with_name("source-manifest.py")))


class Inputs(unittest.TestCase):
    def fixture(self, root):
        for name in manifest["FILES"]:
            (root / name).write_text("")
        for name in manifest["TREES"]:
            (root / name).mkdir(parents=True, exist_ok=True)
        return root / "src/lib.rs"

    def test_sql_fixture_is_a_required_build_input(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = self.fixture(root)
            source.write_text(
                'const SQL: &str = include_str!("../tests/fixtures/upstream/query.sql");'
            )
            with self.assertRaisesRegex(ValueError, "uncovered Rust input"):
                manifest["inputs"](root)
            query = root / "tests/fixtures/upstream/query.sql"
            query.parent.mkdir(parents=True, exist_ok=True)
            query.write_text("FROM lineitem")
            self.assertIn(query, manifest["inputs"](root))
            query.unlink()
            with self.assertRaisesRegex(ValueError, "uncovered Rust input"):
                manifest["inputs"](root)

    def test_workspace_sources_vendor_and_configuration_are_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            self.fixture(root).write_text("")
            for name in [
                "examples/declared.rs",
                "filesystem/Cargo.toml",
                "filesystem/src/syscall.rs",
                "vendor/libc/.cargo-checksum.json",
                "vendor/libc/build.rs",
                ".cargo/config.toml",
            ]:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("")
                self.assertIn(path, manifest["inputs"](root))

    def test_documentation_checked_by_the_gate_is_frozen(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            self.fixture(root).write_text("")
            for name in [
                "README.md",
                "THIRD_PARTY.md",
                "docs/testing.md",
                "notes/plan.md",
            ]:
                path = root / name
                path.write_text("# Maintained guide\n")
                self.assertIn(path, manifest["inputs"](root))

    def test_desktop_metadata_is_excluded_and_cannot_be_a_compiled_input(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = self.fixture(root)
            source.write_text("")
            desktop = root / "src/.DS_Store"
            desktop.write_bytes(b"desktop folder metadata")
            self.assertNotIn(desktop, manifest["inputs"](root))
            source.write_text('const DATA: &[u8] = include_bytes!(".DS_Store");')
            with self.assertRaisesRegex(ValueError, "uncovered Rust input"):
                manifest["inputs"](root)

    def test_compiled_code_and_explicit_modules_cannot_escape_the_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = self.fixture(root)
            (root / "omitted.rs").write_text("pub fn omitted() {}")
            for inclusion in [
                'include!("../omitted.rs");',
                '#[path = "../omitted.rs"] mod omitted;',
            ]:
                with self.subTest(inclusion=inclusion):
                    source.write_text(inclusion)
                    with self.assertRaisesRegex(ValueError, "uncovered Rust input"):
                        manifest["inputs"](root)
            recorded = root / "tools/recorded.rs"
            recorded.write_text("pub fn recorded() {}")
            for inclusion in [
                'include ! ("../tools/recorded.rs",);',
                '# [ path = "../tools/recorded.rs" ] mod recorded;',
            ]:
                source.write_text(inclusion)
                self.assertIn(recorded, manifest["inputs"](root))

    def test_nonliteral_inclusion_is_not_silently_missed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = self.fixture(root)
            for inclusion in [
                'include_str!(concat!("a", "b"));',
                'include_bytes ! (concat!("a", "b"));',
                'include!(concat!(env!("OUT_DIR"), "/generated.rs"));',
                '#[path = r"raw.rs"] mod raw;',
            ]:
                with self.subTest(inclusion=inclusion):
                    source.write_text(inclusion)
                    with self.assertRaisesRegex(ValueError, "nonliteral/unrecognized"):
                        manifest["inputs"](root)


if __name__ == "__main__":
    unittest.main()
