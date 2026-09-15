#!/usr/bin/env python3
"""Challenge fixture discovery, byte comparison, and fresh generation outputs."""
from contextlib import redirect_stdout
import io
from pathlib import Path
import runpy
import shutil
import tempfile
import unittest

import catalog_fixtures

TOOLS = Path(__file__).resolve().parent
CHECK = runpy.run_path(str(TOOLS / "check-fixtures.py"))["check_fixtures"]


class FixtureCoverage(unittest.TestCase):
    def test_catalog_schema_vectors_are_all_checked_without_overwriting(self):
        with tempfile.TemporaryDirectory() as directory:
            fixtures = Path(directory) / "fixtures"
            shutil.copytree(
                TOOLS.parent / "tests/fixtures", fixtures, copy_function=shutil.copyfile
            )
            with redirect_stdout(io.StringIO()) as output:
                CHECK(fixtures)
            self.assertIn("codec fixtures=44", output.getvalue())
            for name in (
                "columns.bin", "catalog.bin", "native-unit.bin",
                "table-data.bin", "data-catalog.bin",
            ):
                with self.subTest(fixture=name):
                    path = fixtures / "catalog-schema" / name
                    original = path.read_bytes()
                    damaged = bytes([original[0] ^ 1]) + original[1:]
                    path.write_bytes(damaged)
                    with self.assertRaises(AssertionError):
                        CHECK(fixtures)
                    self.assertEqual(path.read_bytes(), damaged)
                    path.write_bytes(original)

    def test_database_assembly_refuses_existing_directory_or_link(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "database"
            fixtures = TOOLS.parent / "tests/fixtures"
            catalog_fixtures.write_database(output, fixtures)
            original = (output / "CONTROL").read_bytes()
            with self.assertRaises(FileExistsError):
                catalog_fixtures.write_database(output, fixtures)
            self.assertEqual((output / "CONTROL").read_bytes(), original)
            link = Path(directory) / "link"
            missing = Path(directory) / "missing"
            link.symlink_to(missing)
            with self.assertRaises(FileExistsError):
                catalog_fixtures.write_database(link, fixtures)
            self.assertFalse(missing.exists())

    def test_generation_is_exact_and_refuses_existing_output(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "generated"
            with redirect_stdout(io.StringIO()):
                catalog_fixtures.main([str(output)])
            for group, files in catalog_fixtures.vectors().items():
                for name in files:
                    self.assertEqual(
                        (output / group / name).read_bytes(),
                        (TOOLS.parent / "tests/fixtures" / group / name).read_bytes(),
                    )
            with self.assertRaises(FileExistsError):
                catalog_fixtures.main([str(output)])
            link = Path(directory) / "link"
            missing = Path(directory) / "missing"
            link.symlink_to(missing)
            with self.assertRaises(FileExistsError):
                catalog_fixtures.main([str(link)])
            self.assertFalse(missing.exists())


if __name__ == "__main__":
    unittest.main()
