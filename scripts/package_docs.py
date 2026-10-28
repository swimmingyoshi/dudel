"""Bundle tracked documentation dependencies without collecting private research."""
from pathlib import Path
import re
import shutil
import subprocess
from urllib.parse import unquote, urlsplit

LINK = re.compile(r"(?<!!)\[([^\]\n]+)\]\(([^)\n]+)\)")
MAX_FILE = 2 * 1024 * 1024
MAX_TOTAL = 8 * 1024 * 1024


def bundle_docs(repository, destination, tracked=None):
    repository, destination = Path(repository).resolve(), Path(destination).resolve()
    if repository == destination:
        raise ValueError("Documentation staging cannot be the source repository")
    if tracked is None:
        tracked = set(subprocess.check_output(
            ["git", "ls-files", "-z"], cwd=repository).decode("utf-8").split("\0"))
    pending = list(destination.glob("*.md"))
    visited, copied, omitted = set(), set(), set()
    total = 0
    while pending:
        document = pending.pop()
        if document in visited:
            continue
        if document.is_symlink() or not document.resolve().is_relative_to(destination):
            raise ValueError("Documentation source escapes staging")
        visited.add(document)
        relative_document = document.relative_to(destination)
        text = document.read_text(encoding="utf-8-sig")

        def link(match):
            nonlocal total
            label, raw = match.groups()
            target = raw.strip().strip("<>")
            parsed = urlsplit(target)
            if parsed.scheme or parsed.netloc or not parsed.path:
                return match[0]
            relative = relative_document.parent / unquote(parsed.path)
            original = repository / relative
            output = destination / relative
            if (not original.resolve().is_relative_to(repository)
                    or not output.resolve().is_relative_to(destination)
                    or original.is_symlink() or output.is_symlink()):
                raise ValueError(f"Documentation link escapes package: {raw}")
            original = original.resolve()
            output = output.resolve()
            key = original.relative_to(repository).as_posix()
            # Existing build inputs are already bundled by the source packager.
            # Missing references may add only tracked Markdown/report JSON.
            if not output.is_file():
                allowed = (key in tracked and original.is_file()
                           and original.suffix.lower() in (".md", ".json")
                           and original.stat().st_size <= MAX_FILE)
                if not allowed:
                    omitted.add(key)
                    return f"{label} (repository reference, not bundled: `{target}`)"
                total += original.stat().st_size
                if total > MAX_TOTAL:
                    raise ValueError("Documentation dependency bundle exceeds 8 MiB")
                output.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(original, output)
                copied.add(key)
            if output.suffix.lower() == ".md":
                pending.append(output)
            return match[0]

        updated = LINK.sub(link, text)
        if updated != text:
            document.write_text(updated, encoding="utf-8")
    return {"copied": sorted(copied), "repository_only": sorted(omitted),
            "added_bytes": total, "documents_checked": len(visited)}
