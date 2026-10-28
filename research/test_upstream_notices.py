"""Bundled notices must remain tied to exact crate sources and original bytes."""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from upstream_notices import bundled


class BundledNoticeTests(unittest.TestCase):
    def fixture(self, root):
        bundle, crate, output = root / "bundle", root / "crate", root / "output"
        (bundle / "blobs").mkdir(parents=True)
        crate.mkdir()
        manifest = b'[package]\nname = "example"\nversion = "1.0.0"\n'
        (crate / "Cargo.toml").write_bytes(manifest)
        (crate / ".cargo-checksum.json").write_text(json.dumps({"package": "a" * 64}))
        data = b"Original upstream terms\r\nCopyright Example\n"
        digest = hashlib.sha256(data).hexdigest().upper()
        (bundle / "blobs" / digest).write_bytes(data)
        package = {"name": "example", "version": "1.0.0", "repository": "https://github.com/example/repo", "license": "MIT"}
        record = {
            "repository": package["repository"], "declared_license": "MIT", "crate_checksum": "a" * 64,
            "manifest_sha256": hashlib.sha256(manifest).hexdigest().upper(),
            "files": [{"relative": "upstream/LICENSE", "sha256": digest,
                       "url": "https://raw.githubusercontent.com/example/repo/" + "b" * 40 + "/LICENSE",
                       "revision": "b" * 40}],
        }
        index = {"version": 1, "packages": {"example-1.0.0": record}}
        (bundle / "INDEX.json").write_text(json.dumps(index))
        return bundle, crate, output, package, data, index

    def test_restores_exact_bytes_and_provenance_without_a_network_cache(self):
        with tempfile.TemporaryDirectory() as temporary:
            bundle, crate, output, package, data, index = self.fixture(Path(temporary))
            result = bundled(package, crate, output, bundle)
            self.assertEqual(result[0]["path"].read_bytes(), data)
            self.assertEqual(result[0]["revision"], "b" * 40)
            package["version"] = "2.0.0"
            self.assertIsNone(bundled(package, crate, output, bundle))

    def test_rejects_changed_source_corrupt_bytes_and_escaping_paths_before_copy(self):
        for defect in ("source", "manifest", "license", "bytes", "path"):
            with self.subTest(defect=defect), tempfile.TemporaryDirectory() as temporary:
                bundle, crate, output, package, data, index = self.fixture(Path(temporary))
                record = index["packages"]["example-1.0.0"]
                if defect == "source":
                    (crate / ".cargo-checksum.json").write_text(json.dumps({"package": "c" * 64}))
                elif defect == "manifest":
                    (crate / "Cargo.toml").write_bytes(b"different manifest")
                elif defect == "license":
                    package["license"] = "Apache-2.0"
                elif defect == "bytes":
                    (bundle / "blobs" / record["files"][0]["sha256"]).write_bytes(b"changed terms")
                else:
                    record["files"][0]["relative"] = "../outside"
                    (bundle / "INDEX.json").write_text(json.dumps(index))
                with self.assertRaises(ValueError):
                    bundled(package, crate, output, bundle)
                self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
