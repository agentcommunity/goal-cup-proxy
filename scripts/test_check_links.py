#!/usr/bin/env python3
"""Behavioral tests for the Markdown link checker."""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("check-links.py")
SPEC = importlib.util.spec_from_file_location("check_links", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
check_links = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(check_links)


def write_tree(root: Path, files: dict[str, str]) -> None:
    for name, body in files.items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body, encoding="utf-8")


class SlugTests(unittest.TestCase):
    def test_github_slug_rules(self) -> None:
        cases = {
            "Hello, World!": "hello-world",
            "The `goalcup` binary": "the-goalcup-binary",
            "D42: the public repo": "d42-the-public-repo",
            "A -- B": "a----b",
            "snake_case Name": "snake_case-name",
            "[Linked](x.md) text": "linked-text",
            "Ünïcode ok": "ünïcode-ok",
        }
        for heading, slug in cases.items():
            with self.subTest(heading=heading):
                self.assertEqual(check_links.slugify(heading), slug)

    def test_duplicate_headings_are_numbered(self) -> None:
        found = check_links.anchors_of("# Foo\n\n## Foo\n\n## Foo\n")
        self.assertEqual(found, {"foo", "foo-1", "foo-2"})

    def test_headings_in_fences_do_not_count_and_html_anchors_do(self) -> None:
        found = check_links.anchors_of('```\n# Hidden\n```\n\n<a id="manual"></a>\n\nSetext\n---\n')
        self.assertEqual(found, {"manual", "setext"})

    def test_slug_collisions_skip_names_already_used_by_other_headings(self) -> None:
        found = check_links.anchors_of("# foo-1\n\n# foo\n\n# foo\n")
        self.assertEqual(found, {"foo-1", "foo", "foo-2"})

    def test_container_headings_count_but_code_and_script_headings_do_not(self) -> None:
        text = (
            "> # Quoted heading\n"
            "- ## Listed heading\n"
            "<script>\n# Script heading\n<a id=\"script-anchor\"></a>\n</script>\n"
            "```\n# Fenced heading\n<a id=\"fenced-anchor\"></a>\n```\n"
            "<a id=\"manual\"></a>\n"
        )
        self.assertEqual(
            check_links.anchors_of(text),
            {"quoted-heading", "listed-heading", "manual"},
        )

    def test_script_nested_in_an_html_block_does_not_supply_anchors(self) -> None:
        text = (
            "<div>\n<script>\n<a id=\"hidden\"></a>\n</script>\n</div>\n\n"
            "<a id=\"visible\"></a>\n"
        )
        self.assertEqual(check_links.anchors_of(text), {"visible"})

    def test_indented_script_nested_in_an_html_block_does_not_supply_anchors(self) -> None:
        text = (
            "<div>\n"
            "  <script>\n"
            '  <a id="hidden"></a>\n'
            "  </script>\n"
            "</div>\n"
        )
        self.assertEqual(check_links.anchors_of(text), set())

    def test_commented_anchor_nested_in_html_block_does_not_count(self) -> None:
        text = '<div>\n<!-- <a id="hidden"></a> -->\n</div>\n'
        self.assertEqual(check_links.anchors_of(text), set())

    def test_heading_entities_are_unescaped_and_explicit_ids_keep_case(self) -> None:
        text = '# Fish &amp; Chips\n<a id="CaseID"></a>\n'
        self.assertEqual(check_links.anchors_of(text), {"fish--chips", "CaseID"})

    def test_real_anchor_inside_pre_is_preserved(self) -> None:
        text = '<pre>\n<a id="inside-pre"></a>\n</pre>\n'
        self.assertEqual(check_links.anchors_of(text), {"inside-pre"})

    def test_anchor_inside_standalone_indented_code_does_not_count(self) -> None:
        text = '    <a id="hidden"></a>\n'
        self.assertEqual(check_links.anchors_of(text), set())

    def test_deeply_indented_raw_text_inside_html_blocks_does_not_supply_anchors(self) -> None:
        cases = {
            "plain": (
                "<div>\n"
                "        <script>\n"
                '        <a id="hidden"></a>\n'
                "        </script>\n"
                "</div>\n"
            ),
            "list": (
                "- <div>\n"
                "        <script>\n"
                '        <a id="hidden"></a>\n'
                "        </script>\n"
                "  </div>\n"
            ),
        }
        for name, text in cases.items():
            with self.subTest(name=name):
                self.assertEqual(check_links.anchors_of(text), set())

    def test_same_line_nested_raw_text_inside_html_blocks_does_not_supply_anchors(self) -> None:
        for tag in ("script", "style", "textarea"):
            with self.subTest(tag=tag):
                text = (
                    f"<div><{tag}>\n"
                    '<a id="hidden"></a>\n'
                    f"</{tag}>\n"
                    "</div>\n"
                )
                self.assertEqual(check_links.anchors_of(text), set())

    def test_commented_raw_text_openers_do_not_poison_html_block_anchors(self) -> None:
        for tag in ("script", "style", "textarea"):
            cases = {
                "single line": (
                    f"<div>\n<!-- <{tag}> -->\n"
                    '<a id="visible"></a>\n</div>\n'
                ),
                "multiline": (
                    f"<div>\n<!--\n<{tag}>\n-->\n"
                    '<a id="visible"></a>\n</div>\n'
                ),
                "adjacent closer and pair": (
                    f"<div>\n<!-- </{tag}> -->\n"
                    f"<!-- <{tag}></{tag}> -->\n"
                    '<a id="visible"></a>\n</div>\n'
                ),
            }
            for name, text in cases.items():
                with self.subTest(tag=tag, name=name):
                    self.assertEqual(check_links.anchors_of(text), {"visible"})

    def test_same_line_raw_text_close_preserves_suffix_anchor(self) -> None:
        for tag in ("script", "style", "textarea"):
            with self.subTest(tag=tag):
                text = (
                    f"<div><{tag}>raw</{tag}>"
                    '<a id="real"></a></div>\n'
                )
                self.assertEqual(check_links.anchors_of(text), {"real"})

    def test_same_line_suffix_is_rescanned_for_more_raw_text(self) -> None:
        for first, second in (
            ("script", "script"),
            ("script", "style"),
            ("style", "textarea"),
            ("textarea", "script"),
        ):
            with self.subTest(first=first, second=second):
                text = (
                    f"<div><{first}>x</{first}><{second}>\n"
                    + '<a id="phantom"></a>\n'
                    + f'</{second}><a id="real"></a></div>\n'
                )
                self.assertEqual(check_links.anchors_of(text), {"real"})

    def test_comment_marker_inside_raw_text_does_not_poison_suffix_scan(self) -> None:
        for first, second in (
            ("script", "style"),
            ("style", "textarea"),
            ("textarea", "script"),
        ):
            with self.subTest(first=first, second=second):
                text = (
                    f"<div><{first}><!--</{first}><{second}>\n"
                    + '<a id="phantom"></a>\n'
                    + f'</{second}><a id="real"></a></div>\n'
                )
                self.assertEqual(check_links.anchors_of(text), {"real"})

    def test_tab_indented_html_is_code_not_anchor_markup(self) -> None:
        text = '\t<div><a id="phantom"></a></div>\n'
        self.assertEqual(check_links.anchors_of(text), set())

    def test_script_text_in_link_destination_does_not_hide_later_anchor(self) -> None:
        text = '[script](<script>)\n\n<a id="after"></a>\n'
        self.assertEqual(check_links.anchors_of(text), {"after"})


class LinkTests(unittest.TestCase):
    def test_code_and_comments_are_not_links(self) -> None:
        text = "`[x](gone.md)`\n```\n[y](gone.md)\n```\n<!-- [z](gone.md) -->\n[ok](a.md)\n"
        self.assertEqual(check_links.links_of(text), [(6, "a.md")])

    def test_reference_definitions_and_angle_targets(self) -> None:
        text = "[a][r]\n\n[r]: other.md#top\n[b](<with space.md>)\n"
        self.assertEqual(
            check_links.links_of(text), [(3, "other.md#top"), (4, "with space.md")]
        )

    def test_gfm_destinations_are_not_silently_skipped(self) -> None:
        text = (
            "[angle](<missing file.md>)\n"
            "[nested](missing_(one_(two)).md)\n"
            "[escaped](missing\\(escaped\\).md)\n"
            "[multiline](\n  missing-multiline.md\n  \"title\"\n)\n"
            "> [quoted-ref]: <missing reference.md> \"title\"\n"
        )
        self.assertEqual(
            check_links.links_of(text),
            [
                (1, "missing file.md"),
                (2, "missing_(one_(two)).md"),
                (3, "missing(escaped).md"),
                (4, "missing-multiline.md"),
                (8, "missing reference.md"),
            ],
        )

    def test_table_rows_and_cells_are_separate_inline_contexts(self) -> None:
        text = "A ` | B\n--- | ---\n[x](missing.md) | `\n"
        self.assertEqual(check_links.links_of(text), [(3, "missing.md")])

        cross_cell = "A | B\n--- | ---\n` | [x](missing.md) `\n"
        self.assertEqual(check_links.links_of(cross_cell), [(3, "missing.md")])

    def test_one_cell_table_rows_remain_separate_inline_contexts(self) -> None:
        text = "A | B\n--- | ---\n` [x](missing.md)\n`\n"
        self.assertEqual(check_links.links_of(text), [(3, "missing.md")])

    def test_table_requires_matching_header_and_delimiter_cell_counts(self) -> None:
        text = "A ` | B\n--- | --- | ---\n[x](missing.md) | `\n"
        self.assertEqual(check_links.links_of(text), [])

    def test_one_column_tables_work_in_plain_quote_and_list_containers(self) -> None:
        cases = {
            "plain": "| A |\n| --- |\n| ` [x](missing.md) |\n| ` |\n",
            "quote": "> | A |\n> | --- |\n> | ` [x](missing.md) |\n> | ` |\n",
            "list": "- | A |\n  | --- |\n  | ` [x](missing.md) |\n  | ` |\n",
        }
        for name, text in cases.items():
            with self.subTest(name=name):
                self.assertEqual(check_links.links_of(text), [(3, "missing.md")])

    def test_extra_table_body_cells_are_not_rendered_or_checked(self) -> None:
        text = "A | B\n--- | ---\none | two | [x](missing.md)\n"
        self.assertEqual(check_links.links_of(text), [])

    def test_ordered_list_after_table_uses_fresh_block_context(self) -> None:
        text = "A | B\n--- | ---\n2. `\n[x](missing.md)\n`\n"
        self.assertEqual(check_links.links_of(text), [])

    def test_multiline_reference_definitions_are_checked(self) -> None:
        text = (
            "[plain]:\n  missing.md\n"
            "\n"
            "> [angle]:\n>   <missing angle.md>\n>   \"title\"\n"
        )
        self.assertEqual(
            check_links.links_of(text),
            [(1, "missing.md"), (4, "missing angle.md")],
        )

    def test_reference_definition_labels_are_structurally_validated(self) -> None:
        invalid = {
            "empty": "[]: empty.md\n",
            "escaped-only close": "[foo\\]: escaped-close.md\n",
            "nested opener": "[a[b]: nested.md\n",
        }
        for name, text in invalid.items():
            with self.subTest(name=name):
                self.assertEqual(check_links.links_of(text), [])
        self.assertEqual(
            check_links.links_of("[foo\\]]: valid.md\n"),
            [(1, "valid.md")],
        )

    def test_multiline_reference_titles_must_not_cross_blank_lines(self) -> None:
        text = (
            '[valid]: missing-valid.md "a title\n  continued"\n'
            '[invalid]: missing-invalid.md "a title\n\n  continued"\n'
        )
        self.assertEqual(check_links.links_of(text), [(1, "missing-valid.md")])

    def test_malformed_optional_following_reference_title_keeps_destination(self) -> None:
        cases = {
            "trailing text": (
                '[kept]: missing.md\n  "bad" trailing\n[next]: next.md\n'
            ),
            "unterminated": (
                '[kept]: missing.md\n  "unterminated\n[next]: next.md\n'
            ),
        }
        for name, text in cases.items():
            with self.subTest(name=name):
                self.assertEqual(check_links.links_of(text), [(1, "missing.md")])

        same_line = '[invalid]: missing.md "bad" trailing\n'
        self.assertEqual(check_links.links_of(same_line), [])

    def test_reference_definition_does_not_interrupt_a_paragraph(self) -> None:
        text = "paragraph\n[not-a-definition]:\n  missing.md\n"
        self.assertEqual(check_links.links_of(text), [])

    def test_reference_definition_does_not_interrupt_a_list_item_paragraph(self) -> None:
        text = "- paragraph\n  [not-a-definition]: missing.md\n"
        self.assertEqual(check_links.links_of(text), [])

    def test_link_escape_and_multiline_parenthesized_title_rules(self) -> None:
        text = (
            "\\[escaped](not-a-link.md)\n"
            "[bad-space](missing\\ file.md)\n"
            "[titled](missing.md\n  (a multiline\n  title))\n"
            "[kept](missing\\q.md)\n"
        )
        self.assertEqual(
            check_links.links_of(text),
            [(3, "missing.md"), (6, "missing\\q.md")],
        )

    def test_blank_line_inside_inline_destination_is_invalid(self) -> None:
        text = "[not-a-link](\n\nmissing.md)\n"
        self.assertEqual(check_links.links_of(text), [])

    def test_inline_html_attribute_backticks_do_not_hide_links(self) -> None:
        text = '<span title="`">\n[x](missing.md)\n<span title="`">\n'
        self.assertEqual(check_links.links_of(text), [(2, "missing.md")])

    def test_inline_html_scanner_is_quote_aware_and_multiline(self) -> None:
        cases = {
            "quoted greater-than": '<span title="> `">x</span>\n',
            "multiline tag": '<span\n title="`">x</span>\n',
        }
        for name, tag in cases.items():
            with self.subTest(name=name):
                text = f"{tag}[x](missing.md)\nlater ` closer\n"
                self.assertEqual(
                    check_links.links_of(text),
                    [(tag.count("\n") + 1, "missing.md")],
                )

    def test_inline_html_scanner_handles_pi_declaration_and_cdata_only_when_valid(self) -> None:
        valid = {
            "processing instruction": "before <?pi `?> after\n",
            "declaration": "before <!DOCTYPE data `> after\n",
            "CDATA": "before <![CDATA[`]]> after\n",
        }
        for name, prefix in valid.items():
            with self.subTest(name=name):
                text = f"{prefix}[x](missing.md)\nlater ` closer\n"
                self.assertEqual(check_links.links_of(text), [(2, "missing.md")])

        pseudo = "before <i `> after\n[x](missing.md)\nlater ` closer\n"
        self.assertEqual(check_links.links_of(pseudo), [])

    def test_blank_line_invalidates_inline_title(self) -> None:
        text = '[x](missing.md "title\n\ncontinued")\n'
        self.assertEqual(check_links.links_of(text), [])

    def test_five_space_list_padding_does_not_extend_a_code_span(self) -> None:
        text = "-     ` indented code\n  [x](missing.md)\n  `\n"
        self.assertEqual(check_links.links_of(text), [(2, "missing.md")])

    def test_five_space_list_padding_leaves_indented_code_without_links(self) -> None:
        text = "-     [x](missing.md)\n"
        self.assertEqual(check_links.links_of(text), [])

    def test_standalone_space_and_tab_indented_code_do_not_emit_links(self) -> None:
        text = (
            "    [space](missing-space.md)\n"
            "\t[tab](missing-tab.md)\n"
            "[real](missing-real.md)\n"
        )
        self.assertEqual(check_links.links_of(text), [(3, "missing-real.md")])

    def test_tab_expanded_list_continuation_is_not_indented_code(self) -> None:
        text = "- item\n\n \t[x](missing.md)\n"
        self.assertEqual(check_links.links_of(text), [(3, "missing.md")])

    def test_container_tabs_keep_their_absolute_visual_column(self) -> None:
        cases = {
            "bullet one space": (
                "- item\n\n \t[x](missing.md)\n",
                [(3, "missing.md")],
            ),
            "bullet exact indent": (
                "- item\n\n  \t[x](missing.md)\n",
                [(3, "missing.md")],
            ),
            "bullet three spaces": (
                "- item\n\n   \t[x](missing.md)\n",
                [(3, "missing.md")],
            ),
            "bullet code boundary": (
                "- item\n\n    \t[x](missing.md)\n",
                [],
            ),
            "ordered one space": (
                "1. item\n\n \t[x](missing.md)\n",
                [(3, "missing.md")],
            ),
            "ordered exact indent": (
                "1. item\n\n   \t[x](missing.md)\n",
                [(3, "missing.md")],
            ),
            "ordered code boundary": (
                "1. item\n\n    \t[x](missing.md)\n",
                [],
            ),
            "blockquote exact indent": (
                "> first\n\n> \t[x](missing.md)\n",
                [(3, "missing.md")],
            ),
            "blockquote one extra space": (
                "> first\n\n>  \t[x](missing.md)\n",
                [(3, "missing.md")],
            ),
            "blockquote code boundary": (
                "> first\n\n>    \t[x](missing.md)\n",
                [],
            ),
        }
        for name, (text, expected) in cases.items():
            with self.subTest(name=name):
                self.assertEqual(check_links.links_of(text), expected)

    def test_nested_list_tabs_carry_overshoot_between_containers(self) -> None:
        cases = {
            "bullet tab": (
                "- outer\n  - inner\n\n\t[x](missing.md)\n",
                [(4, "missing.md")],
            ),
            "bullet three spaces tab": (
                "- outer\n  - inner\n\n   \t[x](missing.md)\n",
                [(4, "missing.md")],
            ),
            "bullet four spaces tab is code": (
                "- outer\n  - inner\n\n    \t[x](missing.md)\n",
                [],
            ),
            "bullet five spaces tab is code": (
                "- outer\n  - inner\n\n     \t[x](missing.md)\n",
                [],
            ),
            "ordered tab overshoot": (
                "1. outer\n   1. inner\n\n\t  [x](missing.md)\n",
                [(4, "missing.md")],
            ),
            "quote and ordered tab overshoot": (
                "> 1. outer\n>    1. inner\n\n>   \t[x](missing.md)\n",
                [(4, "missing.md")],
            ),
        }
        for name, (text, expected) in cases.items():
            with self.subTest(name=name):
                self.assertEqual(check_links.links_of(text), expected)

    def test_partial_container_match_preserves_the_longest_prefix(self) -> None:
        cases = {
            "ordered then bullet": (
                "1. outer\n   - inner\n\n\t[x](missing.md)\n",
                [(4, "missing.md")],
            ),
            "bullet then ordered": (
                "- outer\n  1. inner\n\n\t[x](missing.md)\n",
                [(4, "missing.md")],
            ),
            "wide ordered outer": (
                "12345. outer\n       - inner\n\n\t   [x](missing.md)\n",
                [(4, "missing.md")],
            ),
            "no container matches": (
                "12345. outer\n       - inner\n\n\t[x](missing.md)\n",
                [],
            ),
            "all containers match": (
                "1. outer\n   - inner\n\n\t [x](missing.md)\n",
                [(4, "missing.md")],
            ),
            "three deep partial match": (
                "- outer\n  1. middle\n     - inner\n\n\t [x](missing.md)\n",
                [(5, "missing.md")],
            ),
            "list quote ordered partial match": (
                "- > 1. inner\n\n\t> [x](missing.md)\n",
                [(3, "missing.md")],
            ),
        }
        for name, (text, expected) in cases.items():
            with self.subTest(name=name):
                self.assertEqual(check_links.links_of(text), expected)

    def test_container_prefix_matrix_uses_absolute_columns(self) -> None:
        cases = {
            # Each expected tuple is (longest matched prefix, content byte,
            # residual columns, whether the remaining content is code).
            "depth 1 bullet tab": (
                [("list", 2)],
                "\tX",
                ([("list", 2)], 1, 2, False),
            ),
            "depth 1 ordered one space tab": (
                [("list", 3)],
                " \tX",
                ([("list", 3)], 2, 1, False),
            ),
            "depth 2 bullet ordered two spaces tab partial": (
                [("list", 2), ("list", 3)],
                "  \tX",
                ([("list", 2)], 2, 0, False),
            ),
            "depth 3 lists three spaces tab partial": (
                [("list", 2), ("list", 3), ("list", 2)],
                "   \tX",
                ([("list", 2)], 2, 0, False),
            ),
            "depth 3 lists four spaces tab all": (
                [("list", 2), ("list", 3), ("list", 2)],
                "    \tX",
                ([("list", 2), ("list", 3), ("list", 2)], 5, 1, False),
            ),
            "depth 3 lists five spaces tab all": (
                [("list", 2), ("list", 3), ("list", 2)],
                "     \tX",
                ([("list", 2), ("list", 3), ("list", 2)], 6, 1, False),
            ),
            "depth 2 ordered bullet tab partial": (
                [("list", 3), ("list", 2)],
                "\tX",
                ([("list", 3)], 1, 1, False),
            ),
            "depth 2 quote bullet all": (
                [("quote", 0), ("list", 2)],
                "> \tX",
                ([("quote", 0), ("list", 2)], 3, 0, False),
            ),
            "depth 3 quote ordered bullet partial": (
                [("quote", 0), ("list", 3), ("list", 2)],
                "> \t X",
                ([("quote", 0), ("list", 3)], 4, 0, False),
            ),
            "depth 3 quote ordered bullet all": (
                [("quote", 0), ("list", 3), ("list", 2)],
                ">   \tX",
                ([("quote", 0), ("list", 3), ("list", 2)], 5, 1, False),
            ),
            "depth 1 bullet four spaces tab code": (
                [("list", 2)],
                "    \tX",
                ([("list", 2)], 2, 0, True),
            ),
            "depth 1 ordered five spaces tab code": (
                [("list", 3)],
                "     \tX",
                ([("list", 3)], 3, 0, True),
            ),
            "depth 1 wide ordered no match": (
                [("list", 7)],
                "\tX",
                ([], 0, 0, True),
            ),
        }
        for name, (containers, line, expected) in cases.items():
            with self.subTest(name=name):
                matched, pos, residual = check_links.content_in_with_indent(
                    line, containers
                )
                self.assertEqual((matched, pos, residual), expected[:3])
                self.assertEqual(
                    check_links.is_indented_code(line, pos, residual),
                    expected[3],
                )

    def test_list_marker_tab_padding_uses_visual_columns(self) -> None:
        cases = {
            "bullet excess": ("-\t  [x](missing.md)\n", []),
            "bullet tab": ("-\t[x](missing.md)\n", [(1, "missing.md")]),
            "bullet space tab": ("- \t[x](missing.md)\n", [(1, "missing.md")]),
            "bullet spaces tab": ("-  \t[x](missing.md)\n", [(1, "missing.md")]),
            "standalone tab": ("\t[x](missing.md)\n", []),
            "standalone space tab": (" \t[x](missing.md)\n", []),
            "ordered boundary": ("1.\t  [x](missing.md)\n", [(1, "missing.md")]),
            "ordered excess": ("1.\t   [x](missing.md)\n", []),
            "ordered tab stop excess": ("1.  \t[x](missing.md)\n", []),
        }
        for name, (text, expected) in cases.items():
            with self.subTest(name=name):
                self.assertEqual(check_links.links_of(text), expected)

    def test_multiline_code_span_hides_ipv6_reference_definition(self) -> None:
        text = (
            "  The probe recorded upstream failures. For Codex: 31 × `connect timeout\n"
            "  [2a06:98c1:3101::6812:202f]:443` and 12 × `tls handshake eof`.\n"
        )
        self.assertEqual(check_links.links_of(text), [])

    def test_multibacktick_span_can_contain_a_shorter_backtick(self) -> None:
        text = "``literal ` and\n[fake](gone.md)``\n[real](present.md)\n"
        self.assertEqual(check_links.links_of(text), [(3, "present.md")])

    def test_unmatched_code_span_opener_does_not_hide_a_later_link(self) -> None:
        text = "An unmatched ` opener.\n\n[real](present.md)\n"
        self.assertEqual(check_links.links_of(text), [(3, "present.md")])

    def test_unmatched_code_span_opener_stops_at_blank_line(self) -> None:
        text = "An unmatched ` opener.\n\n[real](present.md)\n\nLater ` closer.\n"
        self.assertEqual(check_links.links_of(text), [(3, "present.md")])

    def test_unmatched_code_span_opener_stops_at_heading(self) -> None:
        text = "An unmatched ` opener.\n# [real](present.md)\nLater ` closer.\n"
        self.assertEqual(check_links.links_of(text), [(2, "present.md")])

    def test_unmatched_code_span_opener_stops_at_list_item(self) -> None:
        text = "An unmatched ` opener.\n- [real](present.md)\nLater ` closer.\n"
        self.assertEqual(check_links.links_of(text), [(2, "present.md")])

    def test_unmatched_code_span_opener_stops_at_block_quote(self) -> None:
        text = "An unmatched ` opener.\n> [real](present.md)\nLater ` closer.\n"
        self.assertEqual(check_links.links_of(text), [(2, "present.md")])

    def test_quoted_block_starters_stop_an_unmatched_code_span(self) -> None:
        cases = {
            "heading": (
                "> An unmatched ` opener.\n> # [real](present.md)\n> Later ` closer.\n",
                [(2, "present.md")],
            ),
            "list item": (
                "> An unmatched ` opener.\n> - [real](present.md)\n> Later ` closer.\n",
                [(2, "present.md")],
            ),
            "Setext": (
                "> An unmatched ` opener.\n> ---\n> [real](present.md)\n> Later ` closer.\n",
                [(3, "present.md")],
            ),
        }
        for name, (text, expected) in cases.items():
            with self.subTest(name=name):
                self.assertEqual(check_links.links_of(text), expected)

    def test_recursive_quote_prefixes_expose_nested_block_starters(self) -> None:
        cases = {
            "spaced nested heading": (
                "> An unmatched ` opener.\n> > # [real](present.md)\n> Later ` closer.\n"
            ),
            "compact nested list": (
                "> An unmatched ` opener.\n>> - [real](present.md)\n> Later ` closer.\n"
            ),
        }
        for name, text in cases.items():
            with self.subTest(name=name):
                self.assertEqual(check_links.links_of(text), [(2, "present.md")])

    def test_multiline_code_span_inside_one_quoted_paragraph_is_hidden(self) -> None:
        text = "> literal `code\n> [fake](gone.md)`\n> [real](present.md)\n"
        self.assertEqual(check_links.links_of(text), [(3, "present.md")])

    def test_nested_container_starters_stop_an_outer_list_paragraph_span(self) -> None:
        cases = {
            "nested list": (
                "1. An unmatched ` opener.\n"
                "    - [real](present.md)\n"
                "    Later ` closer.\n",
                [(2, "present.md")],
            ),
            "nested block quote": (
                "1. An unmatched ` opener.\n"
                "    > [real](present.md)\n"
                "    Later ` closer.\n",
                [(2, "present.md")],
            ),
            "list inside quoted list": (
                "> 1. An unmatched ` opener.\n"
                ">     - [real](present.md)\n"
                ">     Later ` closer.\n",
                [(2, "present.md")],
            ),
            "list inside quote inside list": (
                "1. An unmatched ` opener.\n"
                "    > - [real](present.md)\n"
                "    > Later ` closer.\n",
                [(2, "present.md")],
            ),
            "heading inside nested list": (
                "1. An unmatched ` outer opener.\n"
                "    - An unmatched ` inner opener.\n"
                "      # [real](present.md)\n"
                "      Later ` inner closer.\n"
                "    Later ` outer closer.\n",
                [(3, "present.md")],
            ),
            "heading inside quote inside list": (
                "1. An unmatched ` outer opener.\n"
                "    > An unmatched ` inner opener.\n"
                "    > # [real](present.md)\n"
                "    > Later ` inner closer.\n"
                "    Later ` outer closer.\n",
                [(3, "present.md")],
            ),
        }
        for name, (text, expected) in cases.items():
            with self.subTest(name=name):
                self.assertEqual(check_links.links_of(text), expected)

    def test_list_paragraph_continuation_keeps_a_multiline_code_span(self) -> None:
        text = (
            "1. literal `code\n"
            "    [fake](gone.md)\n"
            "    ending`\n"
            "\n"
            "[real](present.md)\n"
        )
        self.assertEqual(check_links.links_of(text), [(5, "present.md")])

    def test_unmatched_code_span_opener_stops_at_setext_and_thematic_breaks(self) -> None:
        for marker in ("---", "===", "* * *", "_ _ _"):
            with self.subTest(marker=marker):
                text = f"An unmatched ` opener.\n{marker}\n[real](present.md)\nLater ` closer.\n"
                self.assertEqual(check_links.links_of(text), [(3, "present.md")])

    def test_only_ordered_list_one_interrupts_an_existing_paragraph(self) -> None:
        interrupting = "An opener ` here.\n1. [real](present.md)\nLater ` closer.\n"
        zero_padded_one = "An opener ` here.\n01. [real](present.md)\nLater ` closer.\n"
        continuing = "An opener ` here.\n2. [fake](gone.md)\nLater ` closer.\n"
        self.assertEqual(check_links.links_of(interrupting), [(2, "present.md")])
        self.assertEqual(check_links.links_of(zero_padded_one), [(2, "present.md")])
        self.assertEqual(check_links.links_of(continuing), [])

    def test_ordered_list_interruption_rules_apply_inside_containers(self) -> None:
        cases = {
            "nested start two continues": (
                "- An opener ` here.\n"
                "  2. [fake](gone.md)\n"
                "  Later ` closer.\n",
                [],
            ),
            "nested start one interrupts": (
                "- An opener ` here.\n"
                "  1. [real](present.md)\n"
                "  Later ` closer.\n",
                [(2, "present.md")],
            ),
            "quoted nested start two continues": (
                "> - An opener ` here.\n"
                ">   2. [fake](gone.md)\n"
                ">   Later ` closer.\n",
                [],
            ),
            "quoted nested start one interrupts": (
                "> - An opener ` here.\n"
                ">   1. [real](present.md)\n"
                ">   Later ` closer.\n",
                [(2, "present.md")],
            ),
        }
        for name, (text, expected) in cases.items():
            with self.subTest(name=name):
                self.assertEqual(check_links.links_of(text), expected)

    def test_empty_list_items_do_not_interrupt_an_open_paragraph(self) -> None:
        cases = {
            "unordered": "An opener ` here.\n- \n[fake](gone.md)\nLater ` closer.\n",
            "ordered": "An opener ` here.\n1. \n[fake](gone.md)\nLater ` closer.\n",
            "nested ordered": (
                "- An opener ` here.\n"
                "  1. \n"
                "  [fake](gone.md)\n"
                "  Later ` closer.\n"
            ),
            "quoted nested ordered": (
                "> - An opener ` here.\n"
                ">   1. \n"
                ">   [fake](gone.md)\n"
                ">   Later ` closer.\n"
            ),
        }
        for name, text in cases.items():
            with self.subTest(name=name):
                self.assertEqual(check_links.links_of(text), [])

    def test_html_block_types_one_through_six_stop_an_unmatched_code_span(self) -> None:
        cases = {
            "type 1": (
                "An unmatched ` opener.\n<script></script>\n"
                "[real](present.md)\nLater ` closer.\n",
                [(3, "present.md")],
            ),
            "type 2": (
                "An unmatched ` opener.\n<!-- comment -->\n"
                "[real](present.md)\nLater ` closer.\n",
                [(3, "present.md")],
            ),
            "type 3": (
                "An unmatched ` opener.\n<?processing instruction?>\n"
                "[real](present.md)\nLater ` closer.\n",
                [(3, "present.md")],
            ),
            "type 4": (
                "An unmatched ` opener.\n<!DOCTYPE html>\n"
                "[real](present.md)\nLater ` closer.\n",
                [(3, "present.md")],
            ),
            "type 5": (
                "An unmatched ` opener.\n<![CDATA[data]]>\n"
                "[real](present.md)\nLater ` closer.\n",
                [(3, "present.md")],
            ),
            "type 6 closes only at a blank line": (
                "An unmatched ` opener.\n<div></div>\n"
                "[fake](gone.md)\n\n[real](present.md)\nLater ` closer.\n",
                [(5, "present.md")],
            ),
        }
        for name, (text, expected) in cases.items():
            with self.subTest(name=name):
                self.assertEqual(check_links.links_of(text), expected)

    def test_multiline_html_blocks_keep_backticks_inside_their_lifetime(self) -> None:
        cases = {
            "type 1 script": "<script>\nraw ` opener [fake](gone.md)\n</script>\n",
            "type 1 pre": "<pre>\nraw ` opener [fake](gone.md)\n</pre>\n",
            "type 1 style": "<style>\nraw ` opener [fake](gone.md)\n</style>\n",
            "type 1 textarea": "<textarea>\nraw ` opener [fake](gone.md)\n</textarea>\n",
            "type 2": "<!--\nraw ` opener [fake](gone.md)\n-->\n",
            "type 3": "<?target\nraw ` opener [fake](gone.md)\n?>\n",
            "type 4": "<!DOCTYPE\nraw ` opener [fake](gone.md)\n>\n",
            "type 5": "<![CDATA[\nraw ` opener [fake](gone.md)\n]]>\n",
        }
        for name, html in cases.items():
            with self.subTest(name=name):
                text = f"{html}[real](present.md)\nLater ` closer.\n"
                self.assertEqual(check_links.links_of(text), [(4, "present.md")])

        type_six = (
            "<div>\nraw ` opener\n</div>\n[fake](gone.md)\n\n"
            "[real](present.md)\nLater ` closer.\n"
        )
        self.assertEqual(check_links.links_of(type_six), [(6, "present.md")])

    def test_html_block_starters_are_found_inside_nested_containers(self) -> None:
        cases = {
            "inside list": (
                "1. An unmatched ` opener.\n"
                "    <script></script>\n"
                "    [real](present.md)\n"
                "    Later ` closer.\n"
            ),
            "inside quoted list": (
                "> - An unmatched ` opener.\n"
                ">   <!DOCTYPE html>\n"
                ">   [real](present.md)\n"
                ">   Later ` closer.\n"
            ),
        }
        for name, text in cases.items():
            with self.subTest(name=name):
                self.assertEqual(check_links.links_of(text), [(3, "present.md")])

    def test_html_block_tabs_expand_from_the_active_absolute_column(self) -> None:
        cases = {
            "bullet": (
                "- item\n\n"
                "  \t<div>\n"
                "  [x](missing.md)\n"
                "  \t</div>\n",
                [],
                set(),
            ),
            "ordered": (
                "1. item\n\n"
                "   \t<div>\n"
                "   [x](missing.md)\n"
                "   \t</div>\n",
                [],
                set(),
            ),
            "blockquote": (
                "> item\n\n"
                ">  \t<div>\n"
                ">  [x](missing.md)\n"
                ">  \t</div>\n",
                [],
                set(),
            ),
            "four-column code then paragraph control": (
                "- item\n\n"
                "    \t<div>\n"
                "    [x](missing.md)\n"
                "    \t</div>\n",
                [(4, "missing.md")],
                set(),
            ),
        }
        for name, (text, expected_links, expected_anchors) in cases.items():
            with self.subTest(name=name):
                self.assertEqual(check_links.links_of(text), expected_links)
                self.assertEqual(check_links.anchors_of(text), expected_anchors)

    def test_html_block_tabs_do_not_expose_nested_markdown_anchors(self) -> None:
        cases = {
            "bullet": (
                "- item\n\n"
                "  \t<div>\n"
                "  # hidden\n"
                "  \t</div>\n"
            ),
            "ordered": (
                "1. item\n\n"
                "   \t<div>\n"
                "   # hidden\n"
                "   \t</div>\n"
            ),
            "blockquote": (
                "> item\n\n"
                ">  \t<div>\n"
                ">  # hidden\n"
                ">  \t</div>\n"
            ),
        }
        for name, text in cases.items():
            with self.subTest(name=name):
                self.assertEqual(check_links.anchors_of(text), set())

    def test_multiline_html_lifetimes_respect_outer_containers(self) -> None:
        cases = {
            "list raw tag": (
                "1. <script>\n"
                "    raw ` opener\n"
                "    </script>\n"
                "    [real](present.md)\n"
                "    Later ` closer.\n",
                [(4, "present.md")],
            ),
            "quoted type six": (
                "> <div>\n"
                "> raw ` opener\n"
                "> </div>\n"
                "> [fake](gone.md)\n"
                ">\n"
                "> [real](present.md)\n"
                "> Later ` closer.\n",
                [(6, "present.md")],
            ),
        }
        for name, (text, expected) in cases.items():
            with self.subTest(name=name):
                self.assertEqual(check_links.links_of(text), expected)

    def test_type_six_html_lifetime_is_decided_before_comments_are_blank(self) -> None:
        text = (
            "<div>\n"
            "<!-- comment -->\n"
            "[fake](gone.md)\n"
            "</div>\n"
            "\n"
            "[real](present.md)\n"
        )
        self.assertEqual(check_links.links_of(text), [(6, "present.md")])

    def test_comment_content_keeps_a_list_item_nonempty(self) -> None:
        text = (
            "An opener ` here.\n"
            "- <!-- comment -->\n"
            "  [real](present.md)\n"
            "Later ` closer.\n"
        )
        self.assertEqual(check_links.links_of(text), [(3, "present.md")])

    def test_inline_comment_backticks_do_not_delimit_code_spans(self) -> None:
        text = (
            "Text <!-- ` ignored -->\n"
            "[real](present.md)\n"
            "Later ` unmatched\n"
        )
        self.assertEqual(check_links.links_of(text), [(2, "present.md")])

    def test_raw_html_is_classified_before_fenced_code(self) -> None:
        text = (
            "<script>\n"
            "```\n"
            "</script>\n"
            "[real](present.md)\n"
            "```\n"
        )
        self.assertEqual(check_links.links_of(text), [(4, "present.md")])

    def test_type_two_terminator_is_not_erased_by_a_raw_fence_marker(self) -> None:
        text = (
            "<!--\n"
            "```\n"
            "-->\n"
            "[real](present.md)\n"
            "```\n"
        )
        self.assertEqual(check_links.links_of(text), [(4, "present.md")])

    def test_comment_delimiters_inside_a_fence_stay_literal(self) -> None:
        text = (
            "```\n"
            "<!--\n"
            "```\n"
            "[real](present.md)\n"
            "-->\n"
        )
        self.assertEqual(check_links.links_of(text), [(4, "present.md")])

    def test_comment_delimiters_inside_raw_html_stay_literal(self) -> None:
        text = (
            "<script>\n"
            "<!--\n"
            "</script>\n"
            "[real](present.md)\n"
            "-->\n"
        )
        self.assertEqual(check_links.links_of(text), [(4, "present.md")])

    def test_comment_opener_inside_code_span_stays_literal(self) -> None:
        text = "`<!--` [real](present.md) -->\n"
        self.assertEqual(check_links.links_of(text), [(1, "present.md")])

    def test_earlier_code_span_opener_wins_over_comment_opener(self) -> None:
        text = "`<!-- ` --> [real](present.md) `\n"
        self.assertEqual(check_links.links_of(text), [(1, "present.md")])

    def test_fence_in_list_survives_an_unindented_blank_line(self) -> None:
        text = (
            "- ```\n"
            "  code\n"
            "\n"
            "  [fake](gone.md)\n"
            "  ```\n"
            "  [real](present.md)\n"
        )
        self.assertEqual(check_links.links_of(text), [(6, "present.md")])

    def test_raw_html_in_list_survives_an_unindented_blank_line(self) -> None:
        text = (
            "- <script>\n"
            "  raw\n"
            "\n"
            "  [fake](gone.md)\n"
            "  </script>\n"
            "  [real](present.md)\n"
        )
        self.assertEqual(check_links.links_of(text), [(6, "present.md")])

    def test_fence_in_quoted_list_survives_a_quote_only_blank_line(self) -> None:
        text = (
            "> - ```\n"
            ">   code\n"
            ">\n"
            ">   [fake](gone.md)\n"
            ">   ```\n"
            ">   [real](present.md)\n"
        )
        self.assertEqual(check_links.links_of(text), [(6, "present.md")])

    def test_raw_html_in_quoted_list_survives_a_quote_only_blank_line(self) -> None:
        text = (
            "> - <script>\n"
            ">   raw\n"
            ">\n"
            ">   [fake](gone.md)\n"
            ">   </script>\n"
            ">   [real](present.md)\n"
        )
        self.assertEqual(check_links.links_of(text), [(6, "present.md")])

    def test_fenced_and_indented_code_keep_surrounding_blocks_separate(self) -> None:
        text = (
            "An unmatched ` opener.\n"
            "```\n[fake](fenced.md)\n```\n\n"
            "    [code](indented.md)\n\n"
            "[real](present.md)\nLater ` closer.\n"
        )
        self.assertEqual(
            check_links.links_of(text),
            [(8, "present.md")],
        )

    def test_escaped_backtick_does_not_open_code_span(self) -> None:
        text = "An escaped \\` opener.\n[real](present.md)\nLater ` closer.\n"
        self.assertEqual(check_links.links_of(text), [(2, "present.md")])

    def test_code_span_delimiters_have_equal_maximal_lengths(self) -> None:
        cases = {
            "three-backtick opener and two-backtick closer": (
                "Start ``` opener\n[real](present.md)\nEnd `` closer.\n"
            ),
            "one-backtick opener and two-backtick run": (
                "Start ` opener\n[real](present.md)\nEnd `` run.\n"
            ),
        }
        for name, text in cases.items():
            with self.subTest(name=name):
                self.assertEqual(check_links.links_of(text), [(2, "present.md")])

    def test_many_unmatched_delimiter_lengths_keep_link_visible(self) -> None:
        unmatched = " ".join("`" * length for length in range(1, 257))
        text = f"{unmatched}\n[real](present.md)\n"
        self.assertEqual(check_links.links_of(text), [(2, "present.md")])

    def test_deep_container_prefix_scales_linearly(self) -> None:
        def fastest(depth: int) -> float:
            text = "- " * depth + "content\n"
            samples = []
            for _ in range(3):
                started = time.perf_counter()
                self.assertEqual(check_links.links_of(text), [])
                samples.append(time.perf_counter() - started)
            return min(samples)

        small = fastest(20_000)
        large = fastest(60_000)
        self.assertLess(large, small * 5)


class CheckTests(unittest.TestCase):
    def check(self, files: dict[str, str]) -> list[tuple[str, int, str, str]]:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_tree(root, files)
            return check_links.check(root)

    def test_good_links_pass(self) -> None:
        broken = self.check(
            {
                "README.md": "[doc](docs/a.md#section-one) [dir](docs) [self](#top) "
                "[web](https://example.com/x.md) [mail](mailto:a@b.c) [abs](/docs/a.md)\n"
                "# Top\n",
                "docs/a.md": "# Section One\n[back](../README.md#top)\n",
                "docs/code.rs": "",
            }
        )
        self.assertEqual(broken, [])

    def test_broken_file_and_anchor_are_listed(self) -> None:
        broken = self.check(
            {
                "README.md": "[a](docs/missing.md)\n[b](docs/a.md#nope)\n[c](#nope)\n",
                "docs/a.md": "# Real\n",
            }
        )
        self.assertEqual(
            broken,
            [
                ("README.md", 1, "docs/missing.md", "no such file"),
                ("README.md", 2, "docs/a.md#nope", "no such anchor"),
                ("README.md", 3, "#nope", "no such anchor"),
            ],
        )

    def test_fragments_on_non_markdown_targets_are_not_checked(self) -> None:
        self.assertEqual(
            self.check({"README.md": "[l](src/lib.rs#L10)\n", "src/lib.rs": "x\n"}), []
        )

    def test_lexical_and_encoded_escapes_are_rejected_but_safe_parent_is_allowed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            root = base / "root"
            root.mkdir()
            write_tree(
                root,
                {
                    "README.md": "root\n",
                    "docs/page.md": (
                        "[safe](../README.md)\n"
                        "[plain](../../outside.md)\n"
                        "[encoded](%2e%2e/%2e%2e/outside.md)\n"
                        "[root-relative](/../outside.md)\n"
                    ),
                },
            )
            (base / "outside.md").write_text("outside\n", encoding="utf-8")
            self.assertEqual(
                check_links.check(root),
                [
                    ("docs/page.md", 2, "../../outside.md", "outside root"),
                    (
                        "docs/page.md",
                        3,
                        "%2e%2e/%2e%2e/outside.md",
                        "outside root",
                    ),
                    ("docs/page.md", 4, "/../outside.md", "outside root"),
                ],
            )

    def test_symlink_escape_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            root = base / "root"
            root.mkdir()
            write_tree(root, {"README.md": "[escape](outside-link.md)\n"})
            outside = base / "outside.md"
            outside.write_text("outside\n", encoding="utf-8")
            (root / "outside-link.md").symlink_to(outside)
            self.assertEqual(
                check_links.check(root),
                [("README.md", 1, "outside-link.md", "outside root")],
            )

    def test_every_supported_gfm_destination_is_checked(self) -> None:
        broken = self.check(
            {
                "README.md": (
                    "[angle](<missing file.md>)\n"
                    "[nested](missing_(one_(two)).md)\n"
                    "[escaped](missing\\(escaped\\).md)\n"
                    "[multiline](\n  missing-multiline.md\n  \"title\"\n)\n"
                    "> [quoted-ref]: <missing reference.md> \"title\"\n"
                )
            }
        )
        self.assertEqual(len(broken), 5)
        self.assertTrue(all(reason == "no such file" for _, _, _, reason in broken))

    def test_commented_anchor_cannot_make_a_fragment_link_clean(self) -> None:
        broken = self.check(
            {
                "README.md": "[hidden](target.md#hidden)\n",
                "target.md": '<div>\n<!-- <a id="hidden"></a> -->\n</div>\n',
            }
        )
        self.assertEqual(
            broken,
            [("README.md", 1, "target.md#hidden", "no such anchor")],
        )

    def test_indented_script_anchor_cannot_make_a_fragment_link_clean(self) -> None:
        broken = self.check(
            {
                "README.md": "[hidden](target.md#hidden)\n",
                "target.md": (
                    "<div>\n"
                    "  <script>\n"
                    '  <a id="hidden"></a>\n'
                    "  </script>\n"
                    "</div>\n"
                ),
            }
        )
        self.assertEqual(
            broken,
            [("README.md", 1, "target.md#hidden", "no such anchor")],
        )

    def test_indented_code_and_deep_raw_text_anchors_fail_end_to_end(self) -> None:
        broken = self.check(
            {
                "README.md": (
                    "[code](code.md#hidden)\n"
                    "[html](html.md#hidden)\n"
                    "[list](list.md#hidden)\n"
                ),
                "code.md": '    <a id="hidden"></a>\n',
                "html.md": (
                    "<div>\n"
                    "        <script>\n"
                    '        <a id="hidden"></a>\n'
                    "        </script>\n"
                    "</div>\n"
                ),
                "list.md": (
                    "- <div>\n"
                    "        <script>\n"
                    '        <a id="hidden"></a>\n'
                    "        </script>\n"
                    "  </div>\n"
                ),
            }
        )
        self.assertEqual(
            broken,
            [
                ("README.md", 1, "code.md#hidden", "no such anchor"),
                ("README.md", 2, "html.md#hidden", "no such anchor"),
                ("README.md", 3, "list.md#hidden", "no such anchor"),
            ],
        )

    def test_same_line_nested_raw_text_anchors_fail_end_to_end(self) -> None:
        files = {
            "README.md": "".join(
                f"[{tag}]({tag}.md#hidden)\n"
                for tag in ("script", "style", "textarea")
            )
        }
        for tag in ("script", "style", "textarea"):
            files[f"{tag}.md"] = (
                f"<div><{tag}>\n"
                '<a id="hidden"></a>\n'
                f"</{tag}>\n"
                "</div>\n"
            )
        self.assertEqual(
            self.check(files),
            [
                ("README.md", 1, "script.md#hidden", "no such anchor"),
                ("README.md", 2, "style.md#hidden", "no such anchor"),
                ("README.md", 3, "textarea.md#hidden", "no such anchor"),
            ],
        )

    def test_commented_raw_text_openers_preserve_anchors_end_to_end(self) -> None:
        files = {"README.md": ""}
        for tag in ("script", "style", "textarea"):
            cases = {
                "single": (
                    f"<div>\n<!-- <{tag}> -->\n"
                    '<a id="visible"></a>\n</div>\n'
                ),
                "multiline": (
                    f"<div>\n<!--\n<{tag}>\n-->\n"
                    '<a id="visible"></a>\n</div>\n'
                ),
                "adjacent": (
                    f"<div>\n<!-- </{tag}> -->\n"
                    f"<!-- <{tag}></{tag}> -->\n"
                    '<a id="visible"></a>\n</div>\n'
                ),
            }
            for name, body in cases.items():
                target = f"{tag}-{name}.md"
                files["README.md"] += f"[{tag}-{name}]({target}#visible)\n"
                files[target] = body
        self.assertEqual(self.check(files), [])

    def test_same_line_raw_text_close_preserves_suffix_anchor_end_to_end(self) -> None:
        files = {"README.md": ""}
        for tag in ("script", "style", "textarea"):
            target = f"{tag}.md"
            files["README.md"] += f"[{tag}]({target}#real)\n"
            files[target] = (
                f"<div><{tag}>raw</{tag}>"
                '<a id="real"></a></div>\n'
            )
        self.assertEqual(self.check(files), [])

    def test_repeated_raw_text_and_raw_comments_are_masked_end_to_end(self) -> None:
        files = {"README.md": ""}
        pairs = (
            ("script", "script"),
            ("script", "style"),
            ("style", "textarea"),
            ("textarea", "script"),
        )
        for index, (first, second) in enumerate(pairs):
            target = f"repeated-{index}.md"
            files["README.md"] += f"[phantom]({target}#phantom)\n"
            files[target] = (
                f"<div><{first}>x</{first}><{second}>\n"
                + '<a id="phantom"></a>\n'
                + f"</{second}></div>\n"
            )
        for index, (first, second) in enumerate(pairs[1:]):
            target = f"comment-{index}.md"
            files["README.md"] += f"[phantom]({target}#phantom)\n"
            files[target] = (
                f"<div><{first}><!--</{first}><{second}>\n"
                + '<a id="phantom"></a>\n'
                + f"</{second}></div>\n"
            )
        self.assertEqual(
            self.check(files),
            [
                ("README.md", line, target, "no such anchor")
                for line, target in enumerate(
                    [
                        "repeated-0.md#phantom",
                        "repeated-1.md#phantom",
                        "repeated-2.md#phantom",
                        "repeated-3.md#phantom",
                        "comment-0.md#phantom",
                        "comment-1.md#phantom",
                        "comment-2.md#phantom",
                    ],
                    1,
                )
            ],
        )

    def test_tab_indented_html_anchor_is_ignored_end_to_end(self) -> None:
        broken = self.check(
            {
                "README.md": "[phantom](target.md#phantom)\n",
                "target.md": '\t<div><a id="phantom"></a></div>\n',
            }
        )
        self.assertEqual(
            broken,
            [("README.md", 1, "target.md#phantom", "no such anchor")],
        )

    def test_optional_reference_titles_follow_end_to_end_rules(self) -> None:
        broken = self.check(
            {
                "a.md": '[kept]: missing-a.md\n  "bad" trailing\n[next]: next-a.md\n',
                "b.md": '[kept]: missing-b.md\n  "unterminated\n[next]: next-b.md\n',
                "c.md": '[invalid]: missing-c.md "bad" trailing\n',
            }
        )
        self.assertEqual(
            broken,
            [
                ("a.md", 1, "missing-a.md", "no such file"),
                ("b.md", 1, "missing-b.md", "no such file"),
            ],
        )

    def test_reference_label_validation_applies_end_to_end(self) -> None:
        broken = self.check(
            {
                "empty.md": "[]: missing-empty.md\n",
                "escaped.md": "[foo\\]: missing-escaped.md\n",
                "nested.md": "[a[b]: missing-nested.md\n",
                "valid-source.md": "[foo\\]]: missing-valid.md\n",
            }
        )
        self.assertEqual(
            broken,
            [("valid-source.md", 1, "missing-valid.md", "no such file")],
        )

    def test_inline_html_constructs_follow_end_to_end_masking_rules(self) -> None:
        broken = self.check(
            {
                "a.md": "before <?pi `?> after\n[x](missing-pi.md)\nlater ` closer\n",
                "b.md": (
                    "before <!DOCTYPE data `> after\n"
                    "[x](missing-declaration.md)\n"
                    "later ` closer\n"
                ),
                "c.md": (
                    "before <![CDATA[`]]> after\n"
                    "[x](missing-cdata.md)\n"
                    "later ` closer\n"
                ),
                "d.md": "before <i `> after\n[x](pseudo.md)\nlater ` closer\n",
            }
        )
        self.assertEqual(
            broken,
            [
                ("a.md", 2, "missing-pi.md", "no such file"),
                ("b.md", 2, "missing-declaration.md", "no such file"),
                ("c.md", 2, "missing-cdata.md", "no such file"),
            ],
        )

    def test_invalid_inline_title_and_list_indented_code_do_not_emit_links(self) -> None:
        broken = self.check(
            {
                "title.md": '[x](missing-title.md "title\n\ncontinued")\n',
                "list.md": "-     [x](missing-list.md)\n",
            }
        )
        self.assertEqual(broken, [])

    def test_standalone_indented_code_links_are_ignored_end_to_end(self) -> None:
        broken = self.check(
            {
                "README.md": (
                    "    [space](missing-space.md)\n"
                    "\t[tab](missing-tab.md)\n"
                    "[real](missing-real.md)\n"
                )
            }
        )
        self.assertEqual(
            broken,
            [("README.md", 3, "missing-real.md", "no such file")],
        )

    def test_tab_expanded_list_continuation_is_checked_end_to_end(self) -> None:
        broken = self.check(
            {"README.md": "- item\n\n \t[x](missing.md)\n"}
        )
        self.assertEqual(
            broken,
            [("README.md", 3, "missing.md", "no such file")],
        )

    def test_container_tabs_keep_absolute_columns_end_to_end(self) -> None:
        files = {
            "bullet-one.md": "- item\n\n \t[x](missing-b1.md)\n",
            "bullet-two.md": "- item\n\n  \t[x](missing-b2.md)\n",
            "bullet-three.md": "- item\n\n   \t[x](missing-b3.md)\n",
            "bullet-code.md": "- item\n\n    \t[x](hidden.md)\n",
            "ordered-one.md": "1. item\n\n \t[x](missing-o1.md)\n",
            "ordered-three.md": "1. item\n\n   \t[x](missing-o3.md)\n",
            "ordered-code.md": "1. item\n\n    \t[x](hidden.md)\n",
            "quote-tab.md": "> first\n\n> \t[x](missing-q0.md)\n",
            "quote-one.md": "> first\n\n>  \t[x](missing-q1.md)\n",
            "quote-code.md": "> first\n\n>    \t[x](hidden.md)\n",
        }
        self.assertEqual(
            self.check(files),
            [
                ("bullet-one.md", 3, "missing-b1.md", "no such file"),
                ("bullet-three.md", 3, "missing-b3.md", "no such file"),
                ("bullet-two.md", 3, "missing-b2.md", "no such file"),
                ("ordered-one.md", 3, "missing-o1.md", "no such file"),
                ("ordered-three.md", 3, "missing-o3.md", "no such file"),
                ("quote-one.md", 3, "missing-q1.md", "no such file"),
                ("quote-tab.md", 3, "missing-q0.md", "no such file"),
            ],
        )

    def test_nested_list_tab_overshoot_applies_end_to_end(self) -> None:
        files = {
            "bullet-tab.md": (
                "- outer\n  - inner\n\n\t[x](missing-b0.md)\n"
            ),
            "bullet-three.md": (
                "- outer\n  - inner\n\n   \t[x](missing-b3.md)\n"
            ),
            "bullet-four-code.md": (
                "- outer\n  - inner\n\n    \t[x](hidden.md)\n"
            ),
            "bullet-five-code.md": (
                "- outer\n  - inner\n\n     \t[x](hidden.md)\n"
            ),
            "ordered.md": (
                "1. outer\n   1. inner\n\n\t  [x](missing-o.md)\n"
            ),
            "quote-ordered.md": (
                "> 1. outer\n>    1. inner\n\n>   \t[x](missing-q.md)\n"
            ),
        }
        self.assertEqual(
            self.check(files),
            [
                ("bullet-tab.md", 4, "missing-b0.md", "no such file"),
                ("bullet-three.md", 4, "missing-b3.md", "no such file"),
                ("ordered.md", 4, "missing-o.md", "no such file"),
                ("quote-ordered.md", 4, "missing-q.md", "no such file"),
            ],
        )

    def test_partial_container_prefix_applies_end_to_end(self) -> None:
        files = {
            "ordered-bullet.md": (
                "1. outer\n   - inner\n\n\t[x](missing-ob.md)\n"
            ),
            "bullet-ordered.md": (
                "- outer\n  1. inner\n\n\t[x](missing-bo.md)\n"
            ),
            "wide-ordered.md": (
                "12345. outer\n       - inner\n\n\t   [x](missing-wide.md)\n"
            ),
            "no-match.md": (
                "12345. outer\n       - inner\n\n\t[x](hidden.md)\n"
            ),
            "all-match.md": (
                "1. outer\n   - inner\n\n\t [x](missing-all.md)\n"
            ),
            "three-deep.md": (
                "- outer\n  1. middle\n     - inner\n\n\t [x](missing-three.md)\n"
            ),
            "list-quote-ordered.md": (
                "- > 1. inner\n\n\t> [x](missing-quote.md)\n"
            ),
        }
        self.assertEqual(
            self.check(files),
            [
                ("all-match.md", 4, "missing-all.md", "no such file"),
                ("bullet-ordered.md", 4, "missing-bo.md", "no such file"),
                ("list-quote-ordered.md", 3, "missing-quote.md", "no such file"),
                ("ordered-bullet.md", 4, "missing-ob.md", "no such file"),
                ("three-deep.md", 5, "missing-three.md", "no such file"),
                ("wide-ordered.md", 4, "missing-wide.md", "no such file"),
            ],
        )

    def test_list_marker_tab_padding_applies_end_to_end(self) -> None:
        files = {
            "bullet-excess.md": "-\t  [x](hidden.md)\n",
            "bullet-tab.md": "-\t[x](missing-a.md)\n",
            "bullet-space-tab.md": "- \t[x](missing-b.md)\n",
            "bullet-spaces-tab.md": "-  \t[x](missing-c.md)\n",
            "standalone-tab.md": "\t[x](hidden.md)\n",
            "standalone-space-tab.md": " \t[x](hidden.md)\n",
            "ordered-boundary.md": "1.\t  [x](missing-d.md)\n",
            "ordered-excess.md": "1.\t   [x](hidden.md)\n",
            "ordered-tab-stop.md": "1.  \t[x](hidden.md)\n",
        }
        self.assertEqual(
            self.check(files),
            [
                ("bullet-space-tab.md", 1, "missing-b.md", "no such file"),
                ("bullet-spaces-tab.md", 1, "missing-c.md", "no such file"),
                ("bullet-tab.md", 1, "missing-a.md", "no such file"),
                ("ordered-boundary.md", 1, "missing-d.md", "no such file"),
            ],
        )

    def test_encoded_controls_are_reported_without_path_errors(self) -> None:
        broken = self.check(
            {
                "README.md": (
                    "[nul](bad%00name.md)\n"
                    "[newline](bad%0Aname.md)\n"
                    "[fragment](#bad%7Fanchor)\n"
                )
            }
        )
        self.assertEqual(
            broken,
            [
                ("README.md", 1, "bad%00name.md", "invalid target"),
                ("README.md", 2, "bad%0Aname.md", "invalid target"),
                ("README.md", 3, "#bad%7Fanchor", "invalid target"),
            ],
        )

    def test_explicit_ids_are_case_sensitive_and_heading_slugs_are_exact(self) -> None:
        broken = self.check(
            {
                "README.md": (
                    "[heading](target.md#fish--chips)\n"
                    "[id](target.md#CaseID)\n"
                    "[wrong-id](target.md#caseid)\n"
                    "[wrong-heading](target.md#Fish--Chips)\n"
                ),
                "target.md": '# Fish &amp; Chips\n<a id="CaseID"></a>\n',
            }
        )
        self.assertEqual(
            broken,
            [
                ("README.md", 3, "target.md#caseid", "no such anchor"),
                ("README.md", 4, "target.md#Fish--Chips", "no such anchor"),
            ],
        )

    def test_cli_exit_status_and_listing(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_tree(root, {"a.md": "[x](gone.md)\n"})
            result = subprocess.run(
                [sys.executable, str(SCRIPT), "--root", str(root)],
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 1)
            self.assertIn("a.md:1: gone.md (no such file)", result.stdout)
            self.assertIn("1 broken link(s)", result.stdout)
            write_tree(root, {"a.md": "fine\n"})
            clean = subprocess.run(
                [sys.executable, str(SCRIPT), "--root", str(root)],
                capture_output=True,
                text=True,
            )
            self.assertEqual(clean.returncode, 0)


if __name__ == "__main__":
    unittest.main()
