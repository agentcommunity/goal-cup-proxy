#!/usr/bin/env python3
"""Check every relative Markdown link and #anchor in the tracked *.md files.

    python3 scripts/check-links.py [--root DIR]

With no --root the repository containing this script is checked, over the files `git ls-files`
names. With --root DIR the check runs over every *.md under DIR (an OSS cut tree is a plain
directory, not a repository; `git ls-files` is used instead when DIR is a git work tree).

A link is checked when it is relative (no scheme such as `https:` or `mailto:`, no `//host`). The target path must exist (a file or a directory); when it is a Markdown file and
the link carries a `#fragment`, the fragment must match a heading slug or an explicit
`<a id=...>`/`<a name=...>` in that file. Slugs follow GitHub's rules: lower-case, drop everything
that is not a letter, digit, `_`, `-` or space, turn spaces into `-`, and number repeats
(`foo`, `foo-1`, `foo-2`). Fragments on non-Markdown targets (`#L10` line anchors) are not checked.
Fenced code blocks, inline code spans and HTML comments are ignored. A target starting with `/`
is resolved from the root, as GitHub does.

Exit status: 0 when nothing is broken, 1 when at least one link is, 2 on a usage error. Stdlib only.
"""

# Maintained stdlib-only GFM link/anchor subset; test_check_links.py is its
# conformance corpus because no external parser is available in locked/offline deps.

from __future__ import annotations

import argparse
from html import unescape as html_unescape
import os
import re
import string
import subprocess
import sys
from pathlib import Path
from urllib.parse import unquote

REPO_ROOT = Path(__file__).resolve().parents[1]

FENCE = re.compile(r"(?P<ind> {0,3})(?P<mark>`{3,}|~{3,})")
HTML_COMMENT = re.compile(r"<!--.*?-->", re.S)
HEADING = re.compile(r" {0,3}#{1,6}(?:[ \t]+(?P<t>.*?))?(?:[ \t]+#+)?[ \t]*$")
SETEXT = re.compile(r" {0,3}(?:=+|-+)[ \t]*$")
THEMATIC = re.compile(r" {0,3}(?:(?:\*[ \t]*){3,}|(?:_[ \t]*){3,}|(?:-[ \t]*){3,})$")
TABLE_DELIMITER_CELL = re.compile(r"[ \t]*:?-+:?[ \t]*$")
BULLET_ITEM = re.compile(r" {0,3}[-+*](?P<pad>[ \t]+)")
ORDERED_ITEM = re.compile(r" {0,3}(?P<start>[0-9]{1,9})[.)](?P<pad>[ \t]+)")
HTML_BLOCK_TAG = re.compile(
    r"</?(?:address|article|aside|base|basefont|blockquote|body|caption|center|col|"
    r"colgroup|dd|details|dialog|dir|div|dl|dt|fieldset|figcaption|figure|footer|"
    r"form|frame|frameset|h[1-6]|head|header|hr|html|iframe|legend|li|link|main|"
    r"menu|menuitem|nav|noframes|ol|optgroup|option|p|param|search|section|summary|"
    r"table|tbody|td|tfoot|th|thead|title|tr|track|ul)(?:[ \t]|/?>|$)",
    re.I,
)
HTML_RAW_TAG = re.compile(r"<(?P<tag>script|pre|style|textarea)(?:[ \t]|>|$)", re.I)
HTML_RAW_CLOSE = re.compile(r"</(?P<tag>script|pre|style|textarea)>", re.I)
HTML_RAW_TEXT_TAG = re.compile(r"<(?P<tag>script|style|textarea)(?:[ \t]|>|$)", re.I)
HTML_ANCHOR = re.compile(r"""<a\s[^>]*?\b(?:id|name)\s*=\s*["']([^"']+)["']""", re.I)
HTML_ID = re.compile(r"""<[a-z][a-z0-9]*\s[^>]*?\bid\s*=\s*["']([^"']+)["']""", re.I)
SCHEME = re.compile(r"^[a-zA-Z][a-zA-Z0-9+.-]*:")

Container = tuple[str, int]
HtmlBlock = tuple[str, str | None]
Fence = tuple[str, int]


def advance_column(column: int, char: str) -> int:
    return column + (4 - column % 4 if char == "\t" else 1)


def column_through(line: str, start: int, end: int, column: int = 0) -> int:
    for char in line[start:end]:
        column = advance_column(column, char)
    return column


def quote_end(line: str, pos: int) -> int | None:
    marker = pos
    spaces = 0
    while marker < len(line) and line[marker] == " " and spaces < 4:
        marker += 1
        spaces += 1
    if spaces > 3 or marker >= len(line) or line[marker] != ">":
        return None
    marker += 1
    if marker < len(line) and line[marker] in " \t":
        marker += 1
    return marker


def list_item(
    line: str, pos: int, final_nonspace: int, column: int | None = None
) -> tuple[str, int | None, int, bool, int, int] | None:
    bullet = BULLET_ITEM.match(line, pos)
    ordered = ORDERED_ITEM.match(line, pos)
    match = bullet or ordered
    if match is None:
        return None
    column = column_through(line, 0, pos) if column is None else column
    marker_column = column_through(line, pos, match.start("pad"), column)
    padded_column = column_through(
        line, match.start("pad"), match.end("pad"), marker_column
    )
    padding = padded_column - marker_column
    if padding <= 4:
        content_pos = match.end()
        content_indent = padded_column - column
        excess = 0
    else:
        content_pos = match.start("pad")
        actual_column = marker_column
        wanted_column = marker_column + 1
        while actual_column < wanted_column:
            actual_column = advance_column(actual_column, line[content_pos])
            content_pos += 1
        content_indent = wanted_column - column
        excess = actual_column - wanted_column
    kind = "bullet" if bullet else "ordered"
    start = None if bullet else int(ordered.group("start"))
    return (
        kind,
        start,
        content_pos,
        content_pos < final_nonspace,
        content_indent,
        excess,
    )


def open_containers_with_indent(
    line: str, final_nonspace: int, pos: int = 0
) -> tuple[list[Container], int, int]:
    containers: list[Container] = []
    column = column_through(line, 0, pos)
    residual = 0
    while pos < len(line):
        after_quote = quote_end(line, pos)
        if after_quote is not None:
            containers.append(("quote", 0))
            column = column_through(line, pos, after_quote, column)
            pos = after_quote
            continue
        item = list_item(line, pos, final_nonspace, column)
        if item is None:
            break
        _, _, content_pos, _, content_indent, residual = item
        containers.append(("list", content_indent))
        column += content_indent + residual
        pos = content_pos
        if residual:
            break
    return containers, pos, residual


def open_containers(
    line: str, final_nonspace: int, pos: int = 0
) -> tuple[list[Container], int]:
    containers, content_pos, _ = open_containers_with_indent(
        line, final_nonspace, pos
    )
    return containers, content_pos


def content_in_with_indent(
    line: str, containers: list[Container]
) -> tuple[list[Container], int, int]:
    matched: list[Container] = []
    pos = 0
    column = 0
    required_column = 0
    residual = 0
    for kind, indent in containers:
        if kind == "quote":
            next_pos = quote_end(line, pos)
            if next_pos is None:
                break
            column = column_through(line, pos, next_pos, column)
            required_column = column
            pos = next_pos
            residual = 0
        else:
            indent_end = required_column + indent
            cursor = pos
            while cursor < len(line) and column < indent_end \
                    and line[cursor] in " \t":
                column = advance_column(column, line[cursor])
                cursor += 1
            if column < indent_end:
                break
            pos = cursor
            required_column = indent_end
            residual = column - indent_end
        matched.append((kind, indent))
    return matched, pos, residual


def content_in(line: str, containers: list[Container]) -> tuple[list[Container], int]:
    matched, pos, _ = content_in_with_indent(line, containers)
    return matched, pos


def block_marker_pos(line: str, pos: int) -> int | None:
    """Return a block marker after GFM's optional zero-to-three-space indent."""
    marker = pos
    start_column = column_through(line, 0, pos)
    columns = start_column
    while marker < len(line) and line[marker] in " \t":
        columns = advance_column(columns, line[marker])
        if columns - start_column > 3:
            return None
        marker += 1
    return marker


def is_indented_code(line: str, pos: int, virtual_columns: int = 0) -> bool:
    """Whether whitespace after the active containers reaches four columns."""
    columns = virtual_columns
    absolute_column = column_through(line, 0, pos)
    while pos < len(line):
        char = line[pos]
        if char not in " \t":
            return False
        next_column = advance_column(absolute_column, char)
        columns += next_column - absolute_column
        absolute_column = next_column
        if columns >= 4:
            return True
        pos += 1
    return False


def html_block_start(line: str, pos: int) -> HtmlBlock | None:
    marker = block_marker_pos(line, pos)
    if marker is None:
        return None
    raw_tag = HTML_RAW_TAG.match(line, marker)
    if raw_tag:
        return "tag", raw_tag.group("tag").lower()
    if line.startswith("<!--", marker):
        return "marker", "-->"
    if line.startswith("<?", marker):
        return "marker", "?>"
    if marker + 2 < len(line) and line.startswith("<!", marker) \
            and "A" <= line[marker + 2] <= "Z":
        return "marker", ">"
    if line.startswith("<![CDATA[", marker):
        return "marker", "]]>"
    if HTML_BLOCK_TAG.match(line, marker):
        return "blank", None
    return None


def html_block_ends(
    block: HtmlBlock, line: str, pos: int, final_nonspace: int
) -> bool:
    kind, value = block
    if kind == "blank":
        return pos >= final_nonspace
    if kind == "tag":
        return any(
            close.group("tag").lower() == value
            for close in HTML_RAW_CLOSE.finditer(line, pos)
        )
    assert value is not None
    return line.find(value, pos) >= 0


def inline_html_end(text: str, start: int, end: int) -> int | None:
    """Return the end of one valid GFM inline HTML construct."""
    def letter(char: str) -> bool:
        return "A" <= char <= "Z" or "a" <= char <= "z"

    def alphanumeric(char: str) -> bool:
        return letter(char) or "0" <= char <= "9"

    if text.startswith("<?", start):
        close = text.find("?>", start + 2, end)
        return close + 2 if close >= 0 else None
    if text.startswith("<![CDATA[", start):
        close = text.find("]]>", start + 9, end)
        return close + 3 if close >= 0 else None
    if text.startswith("<!", start) and start + 2 < end \
            and "A" <= text[start + 2] <= "Z":
        cursor = start + 3
        while cursor < end and "A" <= text[cursor] <= "Z":
            cursor += 1
        if cursor < end and text[cursor] not in " \t\r\n>":
            return None
        close = text.find(">", cursor, end)
        return close + 1 if close >= 0 else None

    cursor = start + 1
    closing = cursor < end and text[cursor] == "/"
    if closing:
        cursor += 1
    if cursor >= end or not letter(text[cursor]):
        return None
    cursor += 1
    while cursor < end and (alphanumeric(text[cursor]) or text[cursor] == "-"):
        cursor += 1
    if closing:
        while cursor < end and text[cursor] in " \t\r\n":
            cursor += 1
        return cursor + 1 if cursor < end and text[cursor] == ">" else None

    while cursor < end:
        whitespace = cursor
        while cursor < end and text[cursor] in " \t\r\n":
            cursor += 1
        if cursor < end and text[cursor] == ">":
            return cursor + 1
        if cursor + 1 < end and text.startswith("/>", cursor):
            return cursor + 2
        if cursor == whitespace or cursor >= end \
                or not (letter(text[cursor]) or text[cursor] in "_:"):
            return None
        cursor += 1
        while cursor < end and (
            alphanumeric(text[cursor]) or text[cursor] in "_.:-"
        ):
            cursor += 1
        while cursor < end and text[cursor] in " \t\r\n":
            cursor += 1
        if cursor >= end or text[cursor] != "=":
            continue
        cursor += 1
        while cursor < end and text[cursor] in " \t\r\n":
            cursor += 1
        if cursor >= end:
            return None
        if text[cursor] in "\"'":
            quote = text[cursor]
            cursor += 1
            close = text.find(quote, cursor, end)
            if close < 0:
                return None
            cursor = close + 1
            continue
        value_start = cursor
        while cursor < end and text[cursor] not in " \t\r\n\"'=<>`":
            cursor += 1
        if cursor == value_start:
            return None
    return None


def inline_html_ranges(text: str, start: int, end: int) -> list[tuple[int, int]]:
    """Scan tags, PIs, declarations, and CDATA without regex boundary drift."""
    ranges: list[tuple[int, int]] = []
    pos = start
    while pos < end:
        html_start = text.find("<", pos, end)
        if html_start < 0:
            break
        html_end = inline_html_end(text, html_start, end)
        if html_end is None:
            pos = html_start + 1
            continue
        ranges.append((html_start, html_end))
        pos = html_end
    return ranges


def next_nested_raw_text_opening(
    line: str, start: int, in_comment: bool
) -> tuple[tuple[str, int] | None, bool]:
    """Find the next non-comment raw opener without scanning past it."""
    cursor = start
    while cursor < len(line):
        if in_comment:
            close = line.find("-->", cursor)
            if close < 0:
                return None, True
            cursor = close + 3
            in_comment = False
            continue
        html_start = line.find("<", cursor)
        if html_start < 0:
            return None, False
        if line.startswith("<!--", html_start):
            in_comment = True
            cursor = html_start + 4
            continue
        opening = HTML_RAW_TEXT_TAG.match(line, html_start)
        if opening is not None:
            return (opening.group("tag").lower(), html_start), False
        html_end = inline_html_end(line, html_start, len(line))
        cursor = html_end if html_end is not None else html_start + 1
    return None, in_comment


def blank_comments(text: str) -> str:
    """Empty HTML comments but keep their newlines so line numbers survive."""
    return HTML_COMMENT.sub(lambda m: "\n" * m.group(0).count("\n"), text)


def blank_raw_html_elements(text: str) -> str:
    """Blank raw-text elements, including nested elements in HTML blocks."""
    out = list(text)
    active: str | None = None
    html_block: HtmlBlock | None = None
    html_containers: list[Container] = []
    comment_active = False
    offset = 0
    for physical_line in text.splitlines(keepends=True):
        line_end = offset + len(physical_line)
        line = physical_line.rstrip("\r\n")
        final_nonspace = len(line.rstrip(" \t"))
        containers, content_pos = open_containers(line, final_nonspace)
        inside_html = False
        if html_block is not None:
            matched, content_pos = content_in(line, html_containers)
            container_blank = content_pos >= final_nonspace and all(
                kind == "list" for kind, _ in html_containers[len(matched):]
            )
            if len(matched) == len(html_containers) or container_blank:
                inside_html = True
            else:
                html_block = None
                html_containers = []
                containers, content_pos = open_containers(line, final_nonspace)
        nested_allowed = inside_html
        scan_pos = 0 if active is not None else content_pos
        if active is None and not inside_html:
            marker = block_marker_pos(line, content_pos)
            block = (
                html_block_start(line, content_pos)
                if marker is not None else None
            )
            if block is not None:
                nested_allowed = True
                scan_pos = marker
                if not html_block_ends(
                    block, line, content_pos, final_nonspace
                ):
                    html_block = block
                    html_containers = containers
        while scan_pos < len(line):
            if active is not None:
                closing = next(
                    (
                        match for match in HTML_RAW_CLOSE.finditer(line, scan_pos)
                        if match.group("tag").lower() == active
                    ),
                    None,
                )
                raw_end = closing.end() if closing is not None else len(line)
                for index in range(offset + scan_pos, offset + raw_end):
                    out[index] = " "
                if closing is None:
                    break
                active = None
                scan_pos = closing.end()
                continue
            if not nested_allowed:
                break
            nested, comment_active = next_nested_raw_text_opening(
                line, scan_pos, comment_active
            )
            if nested is None:
                break
            active, scan_pos = nested
        if html_block is not None and html_block_ends(
            html_block, line, content_pos, final_nonspace
        ):
            html_block = None
            html_containers = []
        offset = line_end
    return "".join(out)


def blank_code_spans(text: str, *, preserve_block_html: bool = False) -> str:
    """Blank code spans inside each paragraph or heading, preserving newlines."""
    out = list(text)

    def preserve_html(block: HtmlBlock) -> bool:
        return preserve_block_html and (
            block[0] == "blank" or (block[0] == "tag" and block[1] == "pre")
        )

    def fence_start(line: str, pos: int) -> Fence | None:
        match = FENCE.match(line, pos)
        if match is None:
            return None
        mark = match.group("mark")
        return mark[0], len(mark)

    def fence_ends(fence: Fence, line: str, pos: int, final_nonspace: int) -> bool:
        match = FENCE.match(line, pos)
        if match is None:
            return False
        mark = match.group("mark")
        return (
            mark[0] == fence[0]
            and len(mark) >= fence[1]
            and match.end("mark") >= final_nonspace
        )

    def block_start(
        line: str, pos: int, final_nonspace: int, paragraph_open: bool
    ) -> str | None:
        if HEADING.match(line, pos):
            return "heading"
        item = list_item(line, pos, final_nonspace)
        if item is not None:
            kind, start, _, nonblank, _, _ = item
            if not paragraph_open or (nonblank and (kind == "bullet" or start == 1)):
                return "container"
            return None
        if SETEXT.match(line, pos) or THEMATIC.match(line, pos):
            return "break"
        if html_block_start(line, pos):
            return "html"
        if fence_start(line, pos):
            return "fence"
        if quote_end(line, pos) is not None:
            return "container"
        return None

    def scrub_block(start: int, end: int) -> None:
        runs: list[tuple[int, int, bool]] = []
        html_tags = inline_html_ranges(text, start, end)
        tag_index = 0
        pos = start
        while pos < end:
            run_start = text.find("`", pos, end)
            if run_start < 0:
                break
            run_end = run_start + 1
            while run_end < end and text[run_end] == "`":
                run_end += 1
            slashes = 0
            before = run_start - 1
            while before >= start and text[before] == "\\":
                slashes += 1
                before -= 1
            while tag_index < len(html_tags) and html_tags[tag_index][1] <= run_start:
                tag_index += 1
            if tag_index < len(html_tags) and html_tags[tag_index][0] <= run_start:
                pos = run_end
                continue
            runs.append((run_start, run_end, slashes % 2 == 1))
            pos = run_end

        comment_starts: list[int] = []
        pos = start
        while pos < end:
            comment_start = text.find("<!--", pos, end)
            if comment_start < 0:
                break
            comment_starts.append(comment_start)
            pos = comment_start + 4

        comment_ends: list[int] = []
        pos = start
        while pos < end:
            comment_end = text.find("-->", pos, end)
            if comment_end < 0:
                break
            comment_ends.append(comment_end)
            pos = comment_end + 3

        next_same: list[int | None] = [None] * len(runs)
        following: dict[int, int] = {}
        for index in range(len(runs) - 1, -1, -1):
            run_start, run_end, _ = runs[index]
            length = run_end - run_start
            next_same[index] = following.get(length)
            following[length] = index

        run_index = 0
        comment_index = 0
        comment_end_index = 0
        pos = start
        while True:
            while run_index < len(runs) and runs[run_index][0] < pos:
                run_index += 1
            while comment_index < len(comment_starts) \
                    and comment_starts[comment_index] < pos:
                comment_index += 1
            while comment_end_index < len(comment_ends) \
                    and comment_ends[comment_end_index] < pos:
                comment_end_index += 1

            run_start = runs[run_index][0] if run_index < len(runs) else end
            comment_start = (
                comment_starts[comment_index]
                if comment_index < len(comment_starts)
                else end
            )
            if run_start == end and comment_start == end:
                break

            if comment_start < run_start:
                while comment_end_index < len(comment_ends) \
                        and comment_ends[comment_end_index] < comment_start + 4:
                    comment_end_index += 1
                if comment_end_index == len(comment_ends):
                    pos = comment_start + 4
                    comment_index += 1
                    continue
                comment_end = comment_ends[comment_end_index] + 3
                blank_range(comment_start, comment_end)
                pos = comment_end
                continue

            run_start, run_end, escaped = runs[run_index]
            closer = next_same[run_index]
            if escaped or closer is None:
                pos = run_end
                run_index += 1
                continue
            span_end = runs[closer][1]
            blank_range(run_start, span_end)
            pos = span_end
            run_index = closer + 1

    def table_cells(start: int, end: int, content_pos: int) -> list[tuple[int, int]]:
        row_start = start + content_pos
        row_end = end
        while row_end > row_start and text[row_end - 1] in "\r\n":
            row_end -= 1
        pipes: list[int] = []
        cursor = row_start
        while cursor < row_end:
            pipe = text.find("|", cursor, row_end)
            if pipe < 0:
                break
            slashes = 0
            before = pipe - 1
            while before >= row_start and text[before] == "\\":
                slashes += 1
                before -= 1
            if slashes % 2 == 0:
                pipes.append(pipe)
            cursor = pipe + 1
        if not pipes:
            return [(row_start, row_end)]
        boundaries = [row_start, *[pipe + 1 for pipe in pipes]]
        cells = [(boundaries[i], pipes[i]) for i in range(len(pipes))]
        cells.append((boundaries[-1], row_end))
        if cells and not text[cells[0][0]:cells[0][1]].strip():
            cells.pop(0)
        if cells and not text[cells[-1][0]:cells[-1][1]].strip():
            cells.pop()
        return cells

    def scrub_table_row(
        start: int, end: int, content_pos: int, width: int | None = None
    ) -> None:
        """Scrub each GFM table cell as its own inline parsing context."""
        for index, (cell_start, cell_end) in enumerate(
            table_cells(start, end, content_pos)
        ):
            if width is not None and index >= width:
                blank_range(cell_start, cell_end)
            else:
                scrub_block(cell_start, cell_end)

    def is_table_delimiter(start: int, end: int, content_pos: int) -> bool:
        row = text[start:end].rstrip("\r\n")
        content = row[content_pos:]
        if "|" not in content:
            return False
        cells = table_cells(start, end, content_pos)
        return bool(cells) and all(
            TABLE_DELIMITER_CELL.fullmatch(text[cell_start:cell_end])
            for cell_start, cell_end in cells
        )

    def blank_range(start: int, end: int) -> None:
        for char_index in range(start, end):
            if out[char_index] not in "\r\n":
                out[char_index] = " "

    def list_container_blank(
        matched: list[Container],
        containers: list[Container],
        content_pos: int,
        final_nonspace: int,
    ) -> bool:
        return content_pos >= final_nonspace and all(
            kind == "list" for kind, _ in containers[len(matched):]
        )

    paragraph_start: int | None = None
    paragraph_containers: list[Container] = []
    html_block: HtmlBlock | None = None
    html_containers: list[Container] = []
    fence: Fence | None = None
    fence_containers: list[Container] = []
    table_state: tuple[list[Container], int] | None = None
    continuation_containers: list[Container] = []

    def remember_list_context(containers: list[Container]) -> None:
        nonlocal continuation_containers
        continuation_containers = (
            containers if any(kind == "list" for kind, _ in containers) else []
        )

    def finish_block(end: int) -> None:
        nonlocal paragraph_start, paragraph_containers
        if paragraph_start is not None:
            scrub_block(paragraph_start, end)
            remember_list_context(paragraph_containers)
        paragraph_start = None
        paragraph_containers = []

    def start_block(
        line: str,
        start: int,
        end: int,
        final_nonspace: int,
        inherited: list[Container] | None = None,
        content_pos: int = 0,
        virtual_indent: int = 0,
    ) -> None:
        nonlocal paragraph_start, paragraph_containers
        nonlocal html_block, html_containers, fence, fence_containers
        containers, content_pos, marker_excess = open_containers_with_indent(
            line, final_nonspace, content_pos
        )
        virtual_indent += marker_excess
        inherited = inherited or []
        if content_pos >= final_nonspace:
            return
        block_containers = [*inherited, *containers]
        remember_list_context(block_containers)
        if is_indented_code(line, content_pos, virtual_indent):
            blank_range(start, end)
            return
        html = html_block_start(line, content_pos)
        if html is not None:
            if not preserve_html(html):
                blank_range(start, end)
            if not html_block_ends(html, line, content_pos, final_nonspace):
                html_block = html
                html_containers = block_containers
            return
        opening_fence = fence_start(line, content_pos)
        if opening_fence is not None:
            blank_range(start, end)
            fence = opening_fence
            fence_containers = block_containers
            return
        kind = block_start(line, content_pos, final_nonspace, False)
        if kind == "break":
            return
        if kind == "heading":
            scrub_block(start, end)
            return
        paragraph_start = start
        paragraph_containers = block_containers

    offset = 0
    for physical_line in text.splitlines(keepends=True):
        line_end = offset + len(physical_line)
        line = physical_line.rstrip("\r\n")
        final_nonspace = len(line)
        while final_nonspace > 0 and line[final_nonspace - 1] in " \t":
            final_nonspace -= 1
        if html_block is not None:
            inherited, content_pos = content_in(line, html_containers)
            if len(inherited) < len(html_containers) and not list_container_blank(
                inherited, html_containers, content_pos, final_nonspace
            ):
                html_block = None
                html_containers = []
                start_block(
                    line, offset, line_end, final_nonspace, inherited, content_pos
                )
            else:
                if not preserve_html(html_block):
                    blank_range(offset, line_end)
                if html_block_ends(html_block, line, content_pos, final_nonspace):
                    remember_list_context(html_containers)
                    html_block = None
                    html_containers = []
        elif fence is not None:
            inherited, content_pos = content_in(line, fence_containers)
            if len(inherited) < len(fence_containers) and not list_container_blank(
                inherited, fence_containers, content_pos, final_nonspace
            ):
                fence = None
                fence_containers = []
                start_block(
                    line, offset, line_end, final_nonspace, inherited, content_pos
                )
            else:
                blank_range(offset, line_end)
                if fence_ends(fence, line, content_pos, final_nonspace):
                    remember_list_context(fence_containers)
                    fence = None
                    fence_containers = []
        elif table_state is not None:
            table_containers, table_width = table_state
            inherited, content_pos = content_in(line, table_containers)
            kind = block_start(line, content_pos, final_nonspace, False)
            if content_pos >= final_nonspace or len(inherited) < len(table_containers) \
                    or kind is not None:
                table_state = None
                start_block(
                    line, offset, line_end, final_nonspace, inherited, content_pos
                )
            else:
                scrub_table_row(offset, line_end, content_pos, table_width)
        elif paragraph_start is None:
            if continuation_containers and final_nonspace > 0:
                inherited, content_pos, virtual_indent = content_in_with_indent(
                    line, continuation_containers
                )
                if len(inherited) < len(continuation_containers):
                    continuation_containers = []
                start_block(
                    line, offset, line_end, final_nonspace,
                    inherited, content_pos, virtual_indent,
                )
            else:
                start_block(line, offset, line_end, final_nonspace)
        else:
            inherited, content_pos = content_in(line, paragraph_containers)
            kind = block_start(line, content_pos, final_nonspace, True)
            if content_pos >= final_nonspace:
                finish_block(offset)
            elif is_table_delimiter(offset, line_end, content_pos) \
                    and paragraph_start == text.rfind("\n", 0, max(0, offset - 1)) + 1:
                header_line = text[paragraph_start:offset].rstrip("\r\n")
                _, header_pos = open_containers(
                    header_line, len(header_line.rstrip(" \t"))
                )
                header_width = len(table_cells(paragraph_start, offset, header_pos))
                delimiter_width = len(table_cells(offset, line_end, content_pos))
                if header_width == delimiter_width:
                    scrub_table_row(paragraph_start, offset, header_pos, header_width)
                    paragraph_start = None
                    table_state = (inherited, header_width)
                    scrub_table_row(offset, line_end, content_pos, header_width)
            elif kind is not None:
                finish_block(offset)
                start_block(
                    line, offset, line_end, final_nonspace, inherited, content_pos
                )
        offset = line_end
    finish_block(len(text))
    return "".join(out)


def heading_text(raw: str) -> str:
    """The rendered text of a heading, as far as a slug cares."""
    raw = re.sub(r"!\[([^\]]*)\]\([^)]*\)", r"\1", raw)
    raw = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", raw)
    raw = re.sub(r"\[([^\]]*)\]\[[^\]]*\]", r"\1", raw)
    raw = re.sub(r"<[^>]+>", "", raw)
    return html_unescape(raw).strip()


def slugify(raw: str) -> str:
    text = heading_text(raw).lower()
    text = re.sub(r"[^\w\- ]", "", text)
    return text.replace(" ", "-")


def anchors_of(text: str) -> set[str]:
    """Every fragment a link into this Markdown text can use."""
    body = blank_code_spans(text)
    html_body = blank_comments(
        blank_raw_html_elements(blank_code_spans(text, preserve_block_html=True))
    )
    found: set[str] = set()
    used_slugs: set[str] = set()
    next_suffix: dict[str, int] = {}
    lines = body.split("\n")
    raw_lines = text.split("\n")

    def add(heading: str) -> None:
        base = slugify(heading)
        slug = base
        suffix = next_suffix.get(base, 1)
        while slug in used_slugs:
            slug = f"{base}-{suffix}"
            suffix += 1
        next_suffix[base] = suffix
        used_slugs.add(slug)
        found.add(slug)

    for i, line in enumerate(lines):
        final_nonspace = len(line.rstrip(" \t"))
        containers, content_pos = open_containers(line, final_nonspace)
        m = HEADING.match(line, content_pos)
        if m:
            raw_match = HEADING.match(raw_lines[i], content_pos)
            add((raw_match or m).group("t") or "")
        elif SETEXT.match(line, content_pos) and i > 0:
            previous = lines[i - 1]
            previous_end = len(previous.rstrip(" \t"))
            previous_containers, previous_pos = open_containers(previous, previous_end)
            previous_text = previous[previous_pos:previous_end]
            if previous_text and previous_containers == containers \
                    and not HEADING.match(previous, previous_pos) \
                    and not SETEXT.match(previous, previous_pos):
                add(raw_lines[i - 1][previous_pos:previous_end])
    for m in HTML_ANCHOR.finditer(html_body):
        found.add(m.group(1))
    for m in HTML_ID.finditer(html_body):
        found.add(m.group(1))
    return found


def unescape_destination(value: str) -> str:
    """Remove Markdown backslash escapes from a destination."""
    return re.sub(r"\\([!\"#$%&'()*+,\-./:;<=>?@\[\\\]^_`{|}~])", r"\1", value)


def skip_link_space(text: str, pos: int) -> int | None:
    newline_seen = False
    while pos < len(text) and text[pos] in " \t\r\n":
        if text[pos] in "\r\n":
            if text[pos] == "\n" or pos + 1 >= len(text) or text[pos + 1] != "\n":
                if newline_seen:
                    return None
                newline_seen = True
        pos += 1
    return pos


def parse_destination(text: str, pos: int) -> tuple[str, int] | None:
    """Parse one GFM-style link destination starting at or after whitespace."""
    pos = skip_link_space(text, pos)
    if pos is None or pos >= len(text):
        return None
    if text[pos] == "<":
        start = pos + 1
        pos = start
        while pos < len(text):
            if text[pos] == "\\" and pos + 1 < len(text):
                pos += 2
                continue
            if text[pos] == ">":
                return unescape_destination(text[start:pos]), pos + 1
            if text[pos] in "\r\n":
                return None
            pos += 1
        return None

    chars: list[str] = []
    depth = 0
    while pos < len(text):
        char = text[pos]
        if char == "\\" and pos + 1 < len(text):
            if text[pos + 1] in string.punctuation:
                chars.append(text[pos + 1])
                pos += 2
            else:
                chars.append(char)
                pos += 1
            continue
        if char == "(":
            depth += 1
            chars.append(char)
            pos += 1
            continue
        if char == ")":
            if depth == 0:
                break
            depth -= 1
            chars.append(char)
            pos += 1
            continue
        if char.isspace():
            break
        if char == "<":
            return None
        chars.append(char)
        pos += 1
    if not chars or depth != 0:
        return None
    return "".join(chars), pos


def parse_title(text: str, pos: int) -> int | None:
    opener = text[pos]
    closer = ")" if opener == "(" else opener
    pos += 1
    while pos < len(text):
        if text[pos] == "\\" and pos + 1 < len(text):
            pos += 2
            continue
        if text[pos] == closer:
            return pos + 1
        if text[pos] in "\r\n":
            next_line = pos + 1
            if text[pos] == "\r" and next_line < len(text) \
                    and text[next_line] == "\n":
                next_line += 1
            probe = next_line
            while probe < len(text) and text[probe] in " \t":
                probe += 1
            if probe < len(text) and text[probe] in "\r\n":
                return None
        pos += 1
    return None


def parse_inline_destination(text: str, pos: int) -> tuple[str, int] | None:
    parsed = parse_destination(text, pos)
    if parsed is None:
        return None
    target, pos = parsed
    if pos < len(text) and text[pos] == ")":
        return target, pos + 1
    if pos >= len(text) or not text[pos].isspace():
        return None
    pos = skip_link_space(text, pos)
    if pos is None:
        return None
    if pos < len(text) and text[pos] in "\"'(":
        title_end = parse_title(text, pos)
        if title_end is None:
            return None
        pos = skip_link_space(text, title_end)
        if pos is None:
            return None
    if pos >= len(text) or text[pos] != ")":
        return None
    return target, pos + 1


def is_escaped(text: str, pos: int) -> bool:
    slashes = 0
    pos -= 1
    while pos >= 0 and text[pos] == "\\":
        slashes += 1
        pos -= 1
    return slashes % 2 == 1


def reference_label_end(line: str, content_pos: int) -> int | None:
    pos = content_pos
    spaces = 0
    while pos < len(line) and line[pos] == " " and spaces < 4:
        pos += 1
        spaces += 1
    if spaces > 3 or pos >= len(line) or line[pos] != "[":
        return None
    pos += 1
    length = 0
    has_nonspace = False
    while pos < len(line):
        char = line[pos]
        if char == "\\" and pos + 1 < len(line) \
                and line[pos + 1] in string.punctuation:
            length += 1
            has_nonspace = has_nonspace or not line[pos + 1].isspace()
            pos += 2
            continue
        if char == "[":
            return None
        if char == "]":
            if pos + 1 >= len(line) or line[pos + 1] != ":":
                return None
            return pos + 2 if length <= 999 and has_nonspace else None
        length += 1
        has_nonspace = has_nonspace or not char.isspace()
        if length > 999:
            return None
        pos += 1
    return None


def continuation_pos(line: str, containers: list[Container]) -> int | None:
    matched, pos = content_in(line, containers)
    if len(matched) < len(containers):
        return None
    spaces = 0
    while pos < len(line) and line[pos] == " " and spaces < 4:
        pos += 1
        spaces += 1
    if spaces > 3:
        return None
    return pos


def reference_title_end(
    lines: list[str], index: int, pos: int, containers: list[Container]
) -> int | None:
    """Parse a nonblank, possibly multiline reference title."""
    opener = lines[index][pos]
    closer = ")" if opener == "(" else opener
    pos += 1
    while True:
        line = lines[index]
        while pos < len(line):
            if line[pos] == "\\" and pos + 1 < len(line):
                pos += 2
                continue
            if line[pos] == closer:
                return index if not line[pos + 1:].strip() else None
            pos += 1
        index += 1
        if index >= len(lines):
            return None
        pos = continuation_pos(lines[index], containers)
        if pos is None or not lines[index][pos:].strip():
            return None


def reference_definition(
    lines: list[str], index: int, containers: list[Container], content_pos: int
) -> tuple[str, int] | None:
    line = lines[index]
    destination_pos = reference_label_end(line, content_pos)
    if destination_pos is None:
        return None
    parsed = parse_destination(line, destination_pos)
    destination_line = index
    if parsed is None and not line[destination_pos:].strip():
        destination_line += 1
        if destination_line >= len(lines):
            return None
        next_pos = continuation_pos(lines[destination_line], containers)
        if next_pos is None:
            return None
        parsed = parse_destination(lines[destination_line], next_pos)
    if parsed is None:
        return None
    target, pos = parsed
    destination_text = lines[destination_line]
    consumed = destination_line
    remainder = destination_text[pos:]
    if remainder.strip():
        if not remainder[0].isspace():
            return None
        title_pos = skip_link_space(destination_text, pos)
        if title_pos is None or title_pos >= len(destination_text) \
                or destination_text[title_pos] not in "\"'(":
            return None
        title_end = reference_title_end(
            lines, destination_line, title_pos, containers
        )
        if title_end is None:
            return None
        consumed = title_end
    elif destination_line + 1 < len(lines):
        title_line = lines[destination_line + 1]
        title_pos = continuation_pos(title_line, containers)
        if title_pos is not None and title_pos < len(title_line) \
                and title_line[title_pos] in "\"'(":
            title_end = reference_title_end(
                lines, destination_line + 1, title_pos, containers
            )
            if title_end is not None:
                consumed = title_end
    return target, consumed


def reference_definitions(body: str) -> list[tuple[int, int, str]]:
    physical_lines = body.splitlines(keepends=True)
    lines = [line.rstrip("\r\n") for line in physical_lines]
    offsets: list[int] = []
    offset = 0
    for line in physical_lines:
        offsets.append(offset)
        offset += len(line)

    found: list[tuple[int, int, str]] = []
    paragraph_open = False
    active_containers: list[Container] = []
    index = 0
    while index < len(lines):
        line = lines[index]
        final_nonspace = len(line.rstrip(" \t"))
        if paragraph_open:
            matched, content_pos = content_in(line, active_containers)
            if len(matched) < len(active_containers):
                paragraph_open = False
            else:
                new_containers, _ = open_containers(line, final_nonspace, content_pos)
                item = list_item(line, content_pos, final_nonspace)
                interrupts = bool(new_containers) and (
                    new_containers[0][0] == "quote"
                    or item is not None
                    and (item[0] == "bullet" or item[1] == 1)
                )
                marker = block_marker_pos(line, content_pos)
                interrupts = interrupts or (
                    marker is not None
                    and (
                        HEADING.match(line, marker)
                        or THEMATIC.match(line, marker)
                        or SETEXT.match(line, marker)
                        or FENCE.match(line, marker)
                        or HTML_BLOCK_TAG.match(line, marker)
                        or HTML_RAW_TAG.match(line, marker)
                    )
                )
                if content_pos >= final_nonspace:
                    paragraph_open = False
                elif not interrupts:
                    index += 1
                    continue
                else:
                    paragraph_open = False
        containers, content_pos = open_containers(line, final_nonspace)
        active_containers = containers
        if content_pos >= final_nonspace:
            paragraph_open = False
            index += 1
            continue
        if not paragraph_open:
            parsed = reference_definition(lines, index, containers, content_pos)
            if parsed is not None:
                target, consumed = parsed
                found.append((offsets[index] + content_pos, index + 1, target))
                index = consumed + 1
                continue
        if HEADING.match(line, content_pos) or THEMATIC.match(line, content_pos) \
                or SETEXT.match(line, content_pos):
            paragraph_open = False
        else:
            paragraph_open = True
        index += 1
    return found


def links_of(text: str) -> list[tuple[int, str]]:
    """(line number, raw target) for every link, skipping code and comments."""
    body = blank_code_spans(text)
    results: list[tuple[int, int, str]] = []
    cursor = 0
    while cursor < len(body):
        close = body.find("](", cursor)
        if close < 0:
            break
        opening = body.rfind("[", 0, close)
        parsed = (
            parse_inline_destination(body, close + 2)
            if opening >= 0 and not is_escaped(body, opening) and not is_escaped(body, close)
            else None
        )
        if parsed is None:
            cursor = close + 2
            continue
        target, end = parsed
        line = body.count("\n", 0, opening) + 1
        results.append((opening, line, target))
        cursor = end

    results.extend(reference_definitions(body))
    return [(line, target) for _, line, target in sorted(results)]


def split_target(target: str) -> tuple[str, str | None]:
    base, _, rest = target.partition("#")
    base = base.split("?", 1)[0]
    return unquote(base), (unquote(rest) if "#" in target else None)


def tracked_markdown(root: Path) -> list[Path]:
    if (root / ".git").exists():
        try:
            names = subprocess.check_output(
                ["git", "-C", str(root), "ls-files", "-z", "--", "*.md"], text=True
            ).split("\0")
        except (OSError, subprocess.CalledProcessError):
            names = []
        if names:
            return sorted(root / n for n in names if n and (root / n).is_file())
    return sorted(
        p for p in root.rglob("*.md") if p.is_file() and ".git" not in p.relative_to(root).parts
    )


def is_within(path: Path, root: Path) -> bool:
    try:
        path.relative_to(root)
    except ValueError:
        return False
    return True


def has_control(value: str | None) -> bool:
    return value is not None and any(ord(char) < 32 or ord(char) == 127 for char in value)


def check(root: Path, files: list[Path] | None = None) -> list[tuple[str, int, str, str]]:
    """Broken links as (file, line, target, reason), file relative to `root`."""
    root = root.resolve()
    files = files if files is not None else tracked_markdown(root)
    anchor_cache: dict[Path, set[str]] = {}
    broken: list[tuple[str, int, str, str]] = []
    for path in files:
        rel_file = path.relative_to(root).as_posix()
        text = path.read_text(encoding="utf-8", errors="replace")
        for line, target in links_of(text):
            if not target or SCHEME.match(target) or target.startswith("//"):
                continue
            base, fragment = split_target(target)
            if has_control(base) or has_control(fragment):
                broken.append((rel_file, line, target, "invalid target"))
                continue
            try:
                if base == "":
                    candidate = path
                elif base.startswith("/"):
                    candidate = root / base.lstrip("/")
                else:
                    candidate = path.parent / base
                lexical = Path(os.path.abspath(candidate))
                dest = lexical.resolve(strict=False)
            except (OSError, ValueError):
                broken.append((rel_file, line, target, "invalid target"))
                continue
            if not is_within(lexical, root):
                broken.append((rel_file, line, target, "outside root"))
                continue
            if not is_within(dest, root):
                broken.append((rel_file, line, target, "outside root"))
                continue
            try:
                if base and not dest.exists():
                    broken.append((rel_file, line, target, "no such file"))
                    continue
                if fragment and dest.is_file() and dest.suffix.lower() == ".md":
                    if dest not in anchor_cache:
                        anchor_cache[dest] = anchors_of(
                            dest.read_text(encoding="utf-8", errors="replace")
                        )
                    if fragment not in anchor_cache[dest]:
                        broken.append((rel_file, line, target, "no such anchor"))
            except (OSError, ValueError):
                broken.append((rel_file, line, target, "invalid target"))
    return broken


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--root", type=Path, default=REPO_ROOT, help="tree to check")
    args = parser.parse_args(argv)
    if not args.root.is_dir():
        print(f"check-links: {args.root} is not a directory", file=sys.stderr)
        return 2
    broken = check(args.root)
    for file, line, target, reason in broken:
        print(f"{file}:{line}: {target} ({reason})")
    print(f"check-links: {len(broken)} broken link(s)")
    return 1 if broken else 0


if __name__ == "__main__":
    sys.exit(main())
