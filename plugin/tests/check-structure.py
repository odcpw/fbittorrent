#!/usr/bin/env python3
"""Reject per-widget polling/processes, ignoring comments and quoted text."""
import re
from pathlib import Path
import sys

TOKENS = re.compile(r'//[^\n]*|/\*[\s\S]*?\*/|"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'')
OBJECT = re.compile(r'\b(?:[A-Za-z_]\w*\.)?(Timer|Process)\s*\{')


def violations(source):
    clean = TOKENS.sub(lambda match: '\n' * match.group().count('\n'), source)
    return [match.group(1) for match in OBJECT.finditer(clean)]


def check(root):
    errors = []
    for path in sorted(root.rglob('*.qml')):
        if path.name != 'Service.qml':
            errors.extend(f'{path}: {kind} must live in Service.qml' for kind in violations(path.read_text()))
    return errors


def self_test():
    assert violations('Timer { interval: 1000 }') == ['Timer']
    assert violations('Io.Process\n{ command: [] }') == ['Process']
    assert not violations('// Timer {}\n/* Process {} */ Text { text: "Timer {" }')
    assert violations('Text {text:"allowed"}\nProcess /* gap */ { }') == ['Process']


if __name__ == '__main__':
    self_test()
    errors = check(Path(sys.argv[1]))
    if errors:
        print('\n'.join(errors), file=sys.stderr)
        sys.exit(1)
    print('Plugin structure: no Timer or Process outside Service.qml; negative controls pass.')
