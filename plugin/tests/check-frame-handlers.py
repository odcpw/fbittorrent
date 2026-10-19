#!/usr/bin/env python3
"""Keep whole-collection JavaScript work out of QML frame handlers."""
import re
from pathlib import Path
import sys
from check_structure_import import cleaned

LOOP=re.compile(r'\b(?:for|while)\s*\(|\.\s*forEach\s*\(')
START=re.compile(r'\b(?:function\s+(?:applyFrame|syncClient|on[A-Z]\w*)\s*\([^)]*\)|on[A-Z]\w*\s*:\s*(?:function\s*\([^)]*\))?)\s*\{')


def violations(source):
    source=cleaned(source);result=[]
    for match in START.finditer(source):
        cursor=match.end();depth=1
        while cursor<len(source) and depth:
            if source[cursor]=='{':depth+=1
            elif source[cursor]=='}':depth-=1
            cursor+=1
        if LOOP.search(source[match.end():cursor-1]):result.append(source.count('\n',0,match.start())+1)
    return result


def self_test():
    assert violations('function applyFrame(frame) { for (const row of rows) patch(row) }')
    assert violations('onFrame: function(frame) { while (rows.length) { rows.pop() } }')
    assert violations('Connections { function onState(frame) { rows.forEach(patch) } }')
    assert not violations('function applyFrame(frame) { store.apply(frame) } // for (;;) {}')
    assert not violations('function applyFrame(frame) { text="while (bad) {}"; /* rows.forEach(f) */ }')
    assert not violations('function advanceWindowFocus() { for(var i=0;i<3;i++){} }')


if __name__=='__main__':
    self_test();root=Path(sys.argv[1]);errors=[]
    for path in [root/'Service.qml',*sorted((root/'views').glob('*.qml'))]:
        errors.extend(f'{path}:{line}: collection loop in frame handler; move it to Store.js' for line in violations(path.read_text()))
    if errors:print('\n'.join(errors),file=sys.stderr);sys.exit(1)
    print('Frame handlers: no for/while/forEach collection loops; positive and negative lint controls pass.')
