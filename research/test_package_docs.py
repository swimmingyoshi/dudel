import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("package_docs", Path(__file__).resolve().parents[1] / "scripts/package_docs.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class DocumentationBundleTests(unittest.TestCase):
    def test_nested_cycles_anchors_and_private_references(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            repo, package = root / "repo", root / "package"
            repo.mkdir(); package.mkdir()
            (repo / "research").mkdir(); (repo / "benchmarks").mkdir()
            text = "[report](research/public.md#details) [private](research/private.md) [web](https://example.com/a)"
            (package / "README.md").write_text(text)
            (repo / "research/public.md").write_text("[home](../README.md) [data](../benchmarks/result.json)")
            (repo / "research/private.md").write_text("PRIVATE CONTENT")
            (repo / "benchmarks/result.json").write_text('{"passed":true}')
            result = module.bundle_docs(repo, package, {"research/public.md", "benchmarks/result.json"})
            self.assertEqual(result["copied"], ["benchmarks/result.json", "research/public.md"])
            self.assertEqual(result["repository_only"], ["research/private.md"])
            self.assertFalse((package / "research/private.md").exists())
            self.assertIn("research/public.md#details", (package / "README.md").read_text())
            self.assertIn("repository reference, not bundled", (package / "README.md").read_text())
            self.assertEqual(result["documents_checked"], 2)

    def test_escape_is_rejected_and_existing_source_inputs_are_unchanged(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            repo, package = root / "repo", root / "package"
            repo.mkdir(); package.mkdir()
            (package / "README.md").write_text("[input](main.rs)")
            (package / "main.rs").write_bytes(b"UNCHANGED\r\n")
            module.bundle_docs(repo, package, set())
            self.assertEqual((package / "main.rs").read_bytes(), b"UNCHANGED\r\n")
            (package / "README.md").write_text("[escape](../outside.md)")
            with self.assertRaisesRegex(ValueError, "escapes"):
                module.bundle_docs(repo, package, {"../outside.md"})

    def test_large_reports_and_artwork_are_not_collected(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            repo, package = root / "repo", root / "package"
            repo.mkdir(); package.mkdir()
            (repo / "huge.json").write_bytes(b"x" * (module.MAX_FILE + 1))
            (repo / "art.dudel").write_bytes(b"ARTWORK")
            (package / "README.md").write_text("[large](huge.json) [art](art.dudel)")
            result = module.bundle_docs(repo, package, {"huge.json", "art.dudel"})
            self.assertEqual(result["copied"], [])
            self.assertEqual(len(result["repository_only"]), 2)
            self.assertFalse((package / "art.dudel").exists())


if __name__ == "__main__":
    unittest.main()
