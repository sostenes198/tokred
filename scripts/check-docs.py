#!/usr/bin/env python3
"""Checks the documentation: bilingual pairs, language switchers, links and a few house rules.

    python3 scripts/check-docs.py

Exit status 0 when everything is fine, 1 otherwise (every problem is listed). Standard library only.

Rules:
  * every English document has a `.pt-BR.md` twin and the other way round;
  * the first line of each is the language switcher pointing at its twin;
  * the twins have the same headings (same levels, same order) and the same number of code blocks;
  * every relative link resolves to a file, and `#anchor`s match a heading of the target;
  * the installer is never compared to another tool, and no document talks about phases or a spec;
  * another tool's name appears only in the one compatibility note (docs/hooks-and-agents);
  * a code block holds one command: the macOS/Linux and Windows installers are never in the same block.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TWIN = ".pt-BR"

# Words a document must not use (case-insensitive), with the reason shown on failure.
FORBIDDEN = [
    (re.compile(r"rustup", re.I), "do not mention (or compare the installer to) that tool"),
    (re.compile(r"\bphases? F\d|\bF\d+\b.{0,3}\bphases?\b", re.I), "no phase vocabulary in user-facing docs"),
    (re.compile(r"\bspec(?:\.pt-BR)?\.md\b"), "the spec no longer exists"),
]
# Another tool's name: allowed only where the compatibility note lives.
NOTE_TOOL = re.compile(r"\brtk\b", re.I)
NOTE_ALLOWED = {"docs/hooks-and-agents.md", "docs/hooks-and-agents.pt-BR.md"}

SKIP_DIRS = {".git", "target", "node_modules", ".claude", ".playwright-mcp"}


def documents():
    for path in sorted(ROOT.rglob("*.md")):
        relative = path.relative_to(ROOT)
        if any(part in SKIP_DIRS for part in relative.parts):
            continue
        # Not documentation: GitHub's templates, and the agent files tokred installs (the product).
        if relative.parts[:1] in {(".github",), ("crates",)}:
            continue
        yield path


def english_of(path):
    name = path.name
    return path.with_name(name.replace(TWIN + ".md", ".md")) if name.endswith(TWIN + ".md") else path


def twin_of(path):
    name = path.name
    if name.endswith(TWIN + ".md"):
        return path.with_name(name[: -len(TWIN + ".md")] + ".md")
    return path.with_name(name[:-3] + TWIN + ".md")


def slug(heading):
    """GitHub's anchor for a heading."""
    text = re.sub(r"`", "", heading.strip().lower())
    text = re.sub(r"[^\w\- ]", "", text, flags=re.UNICODE)
    return text.replace(" ", "-")


def headings(path):
    found = set()
    in_code = False
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("```"):
            in_code = not in_code
        elif not in_code and re.match(r"#{1,6} ", line):
            found.add(slug(line.lstrip("#")))
    return found


def strip_fences(text):
    """The text without the lines inside code blocks (a `# comment` there is not a heading)."""
    out, fence = [], False
    for line in text.splitlines():
        if line.startswith("```"):
            fence = not fence
        elif not fence:
            out.append(line)
    return "\n".join(out)


def code_blocks(text):
    blocks, current, fence = [], None, False
    for line in text.splitlines():
        if line.startswith("```"):
            if fence:
                blocks.append("\n".join(current))
            fence = not fence
            current = []
        elif fence:
            current.append(line)
    return blocks


def check():
    problems = []
    docs = list(documents())

    for path in docs:
        relative = path.relative_to(ROOT).as_posix()
        text = path.read_text(encoding="utf-8")
        is_pt = path.name.endswith(TWIN + ".md")

        # Twin and switcher.
        twin = twin_of(path)
        if not twin.exists():
            problems.append(f"{relative}: missing twin {twin.relative_to(ROOT).as_posix()}")
        first = text.splitlines()[0] if text else ""
        wanted = (
            f"[English]({english_of(path).name}) | **Português**"
            if is_pt
            else f"**English** | [Português]({twin.name})"
        )
        if first != wanted:
            problems.append(f"{relative}: first line must be `{wanted}`")

        # The two languages say the same thing in the same shape (checked from the English side).
        if not is_pt and twin.exists():
            other = twin.read_text(encoding="utf-8")
            shape = lambda t: ([len(m) for m in re.findall(r"^(#{1,6}) ", strip_fences(t), re.M)],
                               code_blocks(t))
            (levels_en, blocks_en), (levels_pt, blocks_pt) = shape(text), shape(other)
            if levels_en != levels_pt:
                problems.append(f"{relative}: the headings differ from the Portuguese twin ({len(levels_en)} vs {len(levels_pt)}, or another order of levels)")
            if len(blocks_en) != len(blocks_pt):
                problems.append(f"{relative}: {len(blocks_en)} code blocks, the Portuguese twin has {len(blocks_pt)}")

        # House rules.
        for pattern, why in FORBIDDEN:
            for number, line in enumerate(text.splitlines(), 1):
                if pattern.search(line):
                    problems.append(f"{relative}:{number}: {why}: {line.strip()[:80]}")
        if relative not in NOTE_ALLOWED:
            for number, line in enumerate(text.splitlines(), 1):
                if NOTE_TOOL.search(line):
                    problems.append(f"{relative}:{number}: another tool's name belongs only in docs/hooks-and-agents: {line.strip()[:80]}")
        for block in code_blocks(text):
            if re.search(r"\bcurl\b.*install\.sh|\bcurl\b.*uninstall\.sh", block) and re.search(r"\birm\b.*\.ps1", block):
                problems.append(f"{relative}: one command per code block (the Linux/macOS and Windows installers share a block)")

        # Links.
        in_code = False
        for number, line in enumerate(text.splitlines(), 1):
            if line.startswith("```"):
                in_code = not in_code
                continue
            if in_code:
                continue
            for target in re.findall(r"(?<!!)\[[^\]]*\]\(([^)\s]+)\)", line):
                if re.match(r"[a-z][a-z0-9+.-]*:", target) or target.startswith("//"):
                    continue  # an address outside the repository
                file_part, _, anchor = target.partition("#")
                resolved = (path.parent / file_part).resolve() if file_part else path
                if file_part and not resolved.exists():
                    problems.append(f"{relative}:{number}: broken link {target}")
                    continue
                if anchor and resolved.suffix == ".md" and slug(anchor) not in headings(resolved) and anchor not in headings(resolved):
                    problems.append(f"{relative}:{number}: no heading for the anchor in {target}")

    return problems


def main():
    problems = check()
    for problem in problems:
        print(problem)
    if problems:
        print(f"\n{len(problems)} documentation problem(s)", file=sys.stderr)
        return 1
    print("documentation ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
