"""Regression for the public-tree privacy check used by verification and packaging."""
from pathlib import Path
import sys
import tempfile
import unittest
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from check_public_tree import archive_members, rules, scan

# Assembled at runtime so this file never contains a real-looking profile path.
PROFILE = "C:" + "\\Users\\" + "someone\\.cargo\\registry"


class PublicTreeTests(unittest.TestCase):
    def setUp(self):
        self.checks = rules(Path("D:/work/dudel"), ["private-note-name"])

    def test_user_profile_paths_in_every_spelling(self):
        for text in (PROFILE, PROFILE.replace("\\", "\\\\"), PROFILE.replace("\\", "/")):
            self.assertEqual(scan(("ok\n" + text).encode(), self.checks), [(2, "user profile path")], text)
        self.assertTrue(scan(PROFILE.encode("utf-16-le"), self.checks))

    def test_placeholders_and_shared_profiles_pass(self):
        for text in ("C:" + "\\Users\\<name>\\AppData", "C:" + "\\Users\\Public\\Documents", "%USERPROFILE%\\.cargo"):
            self.assertEqual(scan(text.encode(), self.checks), [], text)

    def test_checkout_path_and_deny_terms(self):
        self.assertEqual(scan(b'{"exe": "D:\\\\work\\\\dudel\\\\target"}', self.checks), [(1, "checkout path")])
        self.assertEqual(scan(b"see D:/work/dudel/src", self.checks), [(1, "checkout path")])
        self.assertEqual(scan(b"See PRIVATE-NOTE-NAME.md", self.checks), [(1, "deny-list term 'private-note-name'")])
        self.assertTrue(scan("private-note-name".encode("utf-16-le"), self.checks))

    def test_archives_skip_only_third_party_trees(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "dudel-source.zip"
            with zipfile.ZipFile(path, "w") as archive:
                for inner in ("BUILD.json", "vendor/home/src/lib.rs", "third-party-licenses/x.txt", "src/vendor.rs"):
                    archive.writestr("dudel-source/" + inner, PROFILE)
            labels = [label for label, data in archive_members(path) if scan(data, self.checks)]
        self.assertEqual(labels, ["dudel-source.zip:BUILD.json", "dudel-source.zip:src/vendor.rs"])

    def test_without_deny_list_only_built_in_rules_apply(self):
        self.assertEqual(scan(b"private-note-name", rules(Path("D:/work/dudel"), None)), [])


if __name__ == "__main__":
    unittest.main()
