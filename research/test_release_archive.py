"""Regression for Cargo's old timestamps and hidden offline source configuration."""
import os
from pathlib import Path
import sys
import tempfile
import unittest
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from zip_directory import archive
from upstream_notices import notice_path
from package_source import check_thumbnail_dependencies


class ReleaseArchiveTests(unittest.TestCase):
    def test_thumbnail_dependency_drift_cannot_silently_escape_source_and_notices(self):
        source = "registry+https://github.com/rust-lang/crates.io-index"
        metadata = {"packages": [{"name": "image", "version": "0.25.10", "source": source}]}
        with tempfile.TemporaryDirectory() as temporary:
            lock = Path(temporary) / "Cargo.lock"
            covered = (f'[[package]]\nname = "image"\nversion = "0.25.10"\nsource = "{source}"\n'
                       '[[package]]\nname = "dudel_thumbnails"\nversion = "0.1.0"\n')
            lock.write_text(covered, encoding="utf-8")
            check_thumbnail_dependencies(metadata, lock)
            # Same crate name but a different version or origin is not covered.
            for changed in [covered.replace("0.25.10", "0.25.11"),
                            covered.replace(source, "git+https://example.invalid/image")]:
                lock.write_text(changed, encoding="utf-8")
                with self.assertRaisesRegex(ValueError, "Thumbnail dependencies absent"):
                    check_thumbnail_dependencies(metadata, lock)

    def test_font_terms_and_authors_are_not_lost_to_filename_conventions(self):
        for name in ("fonts/OFL.txt", "fonts/UFL.txt", "fonts/Hack-Regular.txt",
                     "fonts/emoji-icon-font-mit-license.txt", "AUTHORS", "LICENSE-MIT"):
            self.assertTrue(notice_path(Path(name)), name)
        self.assertFalse(notice_path(Path("fonts/Ubuntu-Light.ttf")))
        self.assertFalse(notice_path(Path("src/lib.rs")))

    def test_epoch_notice_and_hidden_cargo_configuration_survive(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "candidate"
            config = source / ".cargo" / "config.toml"
            config.parent.mkdir(parents=True)
            config.write_bytes(b'[source.vendored-sources]\ndirectory = "vendor"\n')
            notice = source / "LICENSE"
            notice.write_bytes(b"original upstream notice\n")
            os.utime(notice, (0, 0))
            output = root / "candidate.zip"
            archive(source, output)
            with zipfile.ZipFile(output) as zipped:
                self.assertIsNone(zipped.testzip())
                self.assertEqual(zipped.read("candidate/.cargo/config.toml"), config.read_bytes())
                self.assertEqual(zipped.read("candidate/LICENSE"), notice.read_bytes())
                self.assertEqual(zipped.getinfo("candidate/LICENSE").date_time[0], 1980)

    def test_existing_archive_is_never_replaced(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = root / "candidate.zip"
            output.write_bytes(b"previous candidate")
            with self.assertRaises(FileExistsError):
                archive(root, output)
            self.assertEqual(output.read_bytes(), b"previous candidate")


if __name__ == "__main__":
    unittest.main()
