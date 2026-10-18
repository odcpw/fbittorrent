#!/usr/bin/env python3
"""Check the complete active widget online declaration, including guard fixtures."""
import argparse
from pathlib import Path
import re


ROOT = Path(__file__).resolve().parents[1]
LEX = re.compile(
    r'''(?P<comment>//[^\n]*|/\*.*?(?:\*/|\Z))'''
    r'''|(?P<quoted>"(?:\\.|[^"\\])*(?:"|\Z)|'(?:\\.|[^'\\])*(?:'|\Z)|`(?:\\.|[^`\\])*(?:`|\Z))'''
    r'''|(?P<space>\s+)|(?P<token>===|!==|&&|\|\||[A-Za-z_$][\w$]*|.)''',
    re.DOTALL,
)
PREFIX = ["readonly", "property", "bool", "online", ":"]
VALUE = ["status", ".", "ok", "===", "true", "&&", "status", ".", "running", "!==", "false"]


def online_declaration_ok(source):
    # Ignore comments and keep literals as opaque tokens, so neither can supply
    # an apparently active declaration. Tokens make whitespace immaterial.
    parts = [part for part in LEX.finditer(source) if part.lastgroup in ("quoted", "token")]
    tokens = ["<literal>" if part.lastgroup == "quoted" else part.group() for part in parts]
    starts = [index for index in range(len(tokens))
              if tokens[index:index + len(PREFIX)] == PREFIX and
              (index == 0 or "\n" in source[parts[index - 1].end():parts[index].start()])]
    if len(starts) != 1:
        return False
    value_start = starts[0] + len(PREFIX)
    if tokens[value_start:value_start + len(VALUE)] != VALUE:
        return False
    rest = tokens[value_start + len(VALUE):]
    if rest and rest[0] == ";":
        rest = rest[1:]
    # A complete QML declaration ends at EOF, the object boundary, or the next
    # member. An expression continuation (including || on another line) fails.
    return (not rest or rest[0] in ("}", "readonly", "property", "function", "signal", "enum", "component")
            or len(rest) > 1 and re.fullmatch(r"[A-Za-z_$][\w$]*", rest[0]) is not None
            and rest[1] in (":", "{"))


def self_test():
    declaration = "readonly property bool online: status.ok === true && status.running !== false"
    fixtures = {
        "active": (declaration, True),
        "whitespace": ("\treadonly  property\tbool online :\n status . ok===true\n && status.running !== false ;\n}", True),
        "comment_only": ("// " + declaration + "\nreadonly property bool online: false", False),
        "block_comment_only": ("/*\n" + declaration + "\n*/", False),
        "literal_only": ('property string example: "' + declaration + '"', False),
        "trailing_or_true": (declaration + " || true", False),
        "continued_or_true": (declaration + "\n || true", False),
    }
    passed = True
    for name, (source, expected) in fixtures.items():
        observed = online_declaration_ok(source)
        print(f"widget-online fixture {name}: expect {expected}, observed {observed}")
        passed &= observed == expected
    return passed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", nargs="?", type=Path,
                        default=ROOT / "plugin/oliver.torrents/Service.qml")
    parser.add_argument("--self-test", action="store_true")
    options = parser.parse_args()
    fixtures_ok = not options.self_test or self_test()
    source_ok = online_declaration_ok(options.path.read_text())
    print(f"widget-online active declaration in {options.path}: {'PASS' if source_ok else 'FAIL'}")
    return int(not (fixtures_ok and source_ok))


if __name__ == "__main__":
    raise SystemExit(main())
